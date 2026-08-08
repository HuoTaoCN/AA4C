//! AA4C 设备发现：mDNS 服务注册与浏览。
//!
//! 接口契约见 API_DESIGN.md §5，协议规则见 PROTOCOL.md §1。
//!
//! - 注册 `_aa4c._tcp.local.` 服务，TXT 携带 id/name/platform/ver/proto
//! - 浏览同类服务，过滤自身（id 相同），解析失败的设备静默忽略
//! - 设备离线由 mDNS TTL 过期或注销报文驱动（ServiceRemoved），发布 `DeviceLost`

#![forbid(unsafe_code)]

mod parse;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use aa4c_types::{Aa4cError, CoreEvent, DeviceId, DeviceInfo, Result, SERVICE_TYPE};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use tokio::sync::broadcast;

/// 事件发送端（与 aa4c-core 的事件总线同型）。
pub type EventSender = broadcast::Sender<CoreEvent>;

struct RunState {
    /// 已注册服务的 fullname，用于 stop / rebroadcast 时注销。
    fullname: String,
    /// 实际监听端口，rebroadcast 重新注册时复用。
    port: u16,
    /// 浏览事件处理任务。
    task: tokio::task::JoinHandle<()>,
}

/// mDNS 设备发现服务。
pub struct DiscoveryService {
    daemon: ServiceDaemon,
    /// 本机信息（设备名可经 [`rebroadcast`](DiscoveryService::rebroadcast) 变更）。
    self_info: Mutex<DeviceInfo>,
    events: EventSender,
    /// 当前在线设备快照：DeviceId → DeviceInfo
    devices: Arc<Mutex<HashMap<DeviceId, DeviceInfo>>>,
    /// mDNS fullname → DeviceId（ServiceRemoved 只携带 fullname）
    names: Arc<Mutex<HashMap<String, DeviceId>>>,
    state: Mutex<Option<RunState>>,
}

impl DiscoveryService {
    pub fn new(self_info: DeviceInfo, events: EventSender) -> Result<Self> {
        let daemon = ServiceDaemon::new().map_err(mdns_err)?;
        Ok(Self {
            daemon,
            self_info: Mutex::new(self_info),
            events,
            devices: Arc::new(Mutex::new(HashMap::new())),
            names: Arc::new(Mutex::new(HashMap::new())),
            state: Mutex::new(None),
        })
    }

    /// 注册本机服务（广播）并开始浏览同类服务。
    ///
    /// `listen_port` 为传输服务实际监听端口（端口递增逻辑在传输层，
    /// 此处只负责把真实端口写进 mDNS）。
    pub async fn start(&self, listen_port: u16) -> Result<()> {
        let snapshot = self.self_info.lock().expect("self_info lock").clone();
        {
            let state = self.state.lock().expect("discovery state lock");
            if state.is_some() {
                return Err(Aa4cError::Protocol("discovery already started".into()));
            }
        }

        let fullname = self.register_service(&snapshot, listen_port)?;
        tracing::info!(port = listen_port, "mdns service registered");

        let receiver = self.daemon.browse(SERVICE_TYPE).map_err(mdns_err)?;
        let task = tokio::spawn(browse_loop(
            receiver,
            snapshot.id.clone(),
            self.devices.clone(),
            self.names.clone(),
            self.events.clone(),
        ));

        let mut state = self.state.lock().expect("discovery state lock");
        *state = Some(RunState {
            fullname,
            port: listen_port,
            task,
        });
        Ok(())
    }

    /// 变更本机设备名并重新广播（设置页改名时调用）。
    ///
    /// 未运行时只更新内部名称，下次 [`start`](DiscoveryService::start) 生效；
    /// 名称未变化为 no-op。
    pub async fn rebroadcast(&self, name: String) -> Result<()> {
        let snapshot = {
            let mut info = self.self_info.lock().expect("self_info lock");
            if info.name == name {
                return Ok(());
            }
            info.name = name;
            info.clone()
        };
        let mut state = self.state.lock().expect("discovery state lock");
        let Some(run) = state.as_mut() else {
            return Ok(());
        };
        if let Err(e) = self.daemon.unregister(&run.fullname) {
            tracing::warn!(error = %e, "mdns unregister (rebroadcast) failed");
        }
        run.fullname = self.register_service(&snapshot, run.port)?;
        tracing::info!(name = %snapshot.name, "mdns service re-registered");
        Ok(())
    }

    /// 用当前信息注册 mDNS 服务，返回 fullname。
    fn register_service(&self, info: &DeviceInfo, listen_port: u16) -> Result<String> {
        // 实例名取 DeviceId 前 16 位 hex（PROTOCOL.md §1），足够唯一且可读
        let instance = &info.id[..16];
        let hostname = format!("{instance}.local.");
        let proto = aa4c_types::PROTO_VERSION.to_string();
        let props = [
            (parse::TXT_ID, info.id.as_str()),
            (parse::TXT_NAME, info.name.as_str()),
            (parse::TXT_PLATFORM, info.platform.as_str()),
            (parse::TXT_VERSION, info.version.as_str()),
            (parse::TXT_PROTO, proto.as_str()),
        ];
        let service = ServiceInfo::new(
            SERVICE_TYPE,
            instance,
            &hostname,
            "",
            listen_port,
            &props[..],
        )
        .map_err(mdns_err)?
        .enable_addr_auto();
        let fullname = service.get_fullname().to_string();
        self.daemon.register(service).map_err(mdns_err)?;
        Ok(fullname)
    }

    /// 注销服务并停止浏览。已停止时为幂等 no-op。
    pub async fn stop(&self) -> Result<()> {
        let run = self.state.lock().expect("discovery state lock").take();
        let Some(run) = run else { return Ok(()) };

        if let Err(e) = self.daemon.unregister(&run.fullname) {
            tracing::warn!(error = %e, "mdns unregister failed");
        }
        if let Err(e) = self.daemon.stop_browse(SERVICE_TYPE) {
            tracing::warn!(error = %e, "mdns stop_browse failed");
        }
        run.task.abort();
        self.devices.lock().expect("devices lock").clear();
        self.names.lock().expect("names lock").clear();
        tracing::info!("mdns discovery stopped");
        Ok(())
    }

    /// 当前发现的设备快照（不含本机）。
    pub fn devices(&self) -> Vec<DeviceInfo> {
        self.devices
            .lock()
            .expect("devices lock")
            .values()
            .cloned()
            .collect()
    }
}

impl Drop for DiscoveryService {
    /// 释放 mDNS 守护进程。
    ///
    /// `ServiceDaemon` 自带一个**独立的 OS 线程**和一组 5353 组播 socket，两者都不随
    /// tokio runtime 结束而消亡；[`Self::stop`] 只注销服务、停止浏览（之后仍可 `start`
    /// 重开），刻意不碰守护进程本身。于是"谁都不再持有这个 `DiscoveryService`了"是唯一
    /// 该回收它的时机——也只有放在 `Drop` 里，才能覆盖**调用方没能走到 `stop()`**的路径
    /// （panic、提前 return）。此前这一层缺失：进程内每 `new` 一次就永久多一条线程，
    /// 集成测试整套跑下来会堆到几十条，全都在处理局域网上的每一个 mDNS 包。
    fn drop(&mut self) {
        // best-effort：进程正在退出时 daemon 可能已经没了，报错无意义。
        let _ = self.daemon.shutdown();
    }
}

/// 浏览事件循环：维护设备表并发布 CoreEvent。
async fn browse_loop(
    receiver: mdns_sd::Receiver<ServiceEvent>,
    self_id: DeviceId,
    devices: Arc<Mutex<HashMap<DeviceId, DeviceInfo>>>,
    names: Arc<Mutex<HashMap<String, DeviceId>>>,
    events: EventSender,
) {
    while let Ok(event) = receiver.recv_async().await {
        match event {
            ServiceEvent::ServiceResolved(info) => {
                let Some(device) = parse::parse_service(&info) else {
                    tracing::debug!(
                        fullname = info.get_fullname(),
                        "ignoring unparsable service"
                    );
                    continue;
                };
                if device.id == self_id {
                    continue; // 自身回环
                }
                names
                    .lock()
                    .expect("names lock")
                    .insert(info.get_fullname().to_string(), device.id.clone());

                let previous = devices
                    .lock()
                    .expect("devices lock")
                    .insert(device.id.clone(), device.clone());
                let event = match previous {
                    None => {
                        tracing::info!(id = %device.id, name = %device.name, "device found");
                        CoreEvent::DeviceFound(device)
                    }
                    Some(ref old) if old != &device => CoreEvent::DeviceUpdated(device),
                    Some(_) => continue, // 周期性重解析、无变化：不发事件
                };
                let _ = events.send(event);
            }
            ServiceEvent::ServiceRemoved(_, fullname) => {
                let id = names.lock().expect("names lock").remove(&fullname);
                let Some(id) = id else { continue };
                if devices.lock().expect("devices lock").remove(&id).is_some() {
                    tracing::info!(id = %id, "device lost");
                    let _ = events.send(CoreEvent::DeviceLost { id });
                }
            }
            _ => {}
        }
    }
}

fn mdns_err(e: mdns_sd::Error) -> Aa4cError {
    Aa4cError::Network(format!("mdns: {e}"))
}

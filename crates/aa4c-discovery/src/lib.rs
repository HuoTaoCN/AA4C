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
    /// 已注册服务的 fullname，用于 stop 时注销。
    fullname: String,
    /// 浏览事件处理任务。
    task: tokio::task::JoinHandle<()>,
}

/// mDNS 设备发现服务。
pub struct DiscoveryService {
    daemon: ServiceDaemon,
    self_info: DeviceInfo,
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
            self_info,
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
        {
            let state = self.state.lock().expect("discovery state lock");
            if state.is_some() {
                return Err(Aa4cError::Protocol("discovery already started".into()));
            }
        }

        // 实例名取 DeviceId 前 16 位 hex（PROTOCOL.md §1），足够唯一且可读
        let instance = &self.self_info.id[..16];
        let hostname = format!("{instance}.local.");
        let props = [
            (parse::TXT_ID, self.self_info.id.as_str()),
            (parse::TXT_NAME, self.self_info.name.as_str()),
            (parse::TXT_PLATFORM, self.self_info.platform.as_str()),
            (parse::TXT_VERSION, self.self_info.version.as_str()),
            (parse::TXT_PROTO, "1"),
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
        tracing::info!(port = listen_port, "mdns service registered");

        let receiver = self.daemon.browse(SERVICE_TYPE).map_err(mdns_err)?;
        let task = tokio::spawn(browse_loop(
            receiver,
            self.self_info.id.clone(),
            self.devices.clone(),
            self.names.clone(),
            self.events.clone(),
        ));

        let mut state = self.state.lock().expect("discovery state lock");
        *state = Some(RunState { fullname, task });
        Ok(())
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

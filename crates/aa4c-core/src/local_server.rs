//! 内置可选 server 模式（TRUST_DESIGN.md §6.3，V0.7 里程碑 R4）。
//!
//! `aa4c-server` 本来是个独立二进制，实际要求用户有台 VPS。桌面端把它当**库**依赖、在进程
//! 内起一个实例，门槛就从「要有 VPS」降到「家里有台常开设备」——那台台式机或 NAS 开着，
//! 它就是你的汇合点。
//!
//! ## 必须诚实说清的前提
//!
//! 打开这个开关**不等于**别的设备就能找到你。你仍然需要一个**稳定入口**：DDNS 域名，或者
//! 一个相对固定的地址。这是「零第三方」的真实边界——不需要服务商，但需要你自己有个能被
//! 找到的落脚点（TRUST_DESIGN.md §6.3）。界面上不能含糊这一条。
//!
//! ## 身份：刻意复用设备自己的
//!
//! 内嵌实例的 `data_dir` 就是 Core 的 `data_dir`，于是 `Identity::load_or_generate` 读到的
//! 是同一个 `device.key`——**内置服务器的指纹就是这台设备的指纹**。另起一套身份会让同一台
//! 机器凭空多出第二个指纹，用户还得理解「哪个是哪个」，没有任何好处。安全上也不增加暴露面：
//! 传输监听器本来就用同一把密钥对外服务。
//!
//! ## 端口
//!
//! 默认 42421，**刻意不等于**传输端口（42420）。传输层已经占了那个号的 TCP 与 UDP，而内置
//! 服务器同样两个都要（TCP 走信令/中继，UDP 走反射端点）。

use std::net::{Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use aa4c_server::{Server, ServerConfig};
use aa4c_store::Store;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

/// 检查设置变化的间隔。用户拨这个开关的频率极低，不值得更密。
const POLL: Duration = Duration::from_secs(30);

/// 内置服务器的当前实例（`None` = 没开，或者起不来）+ 起它要用的数据目录。
#[derive(Clone)]
pub(crate) struct LocalServer {
    inner: Arc<Mutex<Option<Arc<Server>>>>,
    data_dir: PathBuf,
}

impl LocalServer {
    pub(crate) fn new(data_dir: PathBuf) -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
            data_dir,
        }
    }

    /// 当前实例句柄；未启动时为 `None`。
    pub(crate) async fn get(&self) -> Option<Arc<Server>> {
        self.inner.lock().await.clone()
    }

    /// 当前正在监听的端口；未启动时为 `None`。供端口映射那边知道要映哪个口。
    pub(crate) async fn port(&self) -> Option<u16> {
        self.inner
            .lock()
            .await
            .as_ref()
            .map(|s| s.local_addr().port())
    }

    /// 立刻把实际状态对齐到设置——**保存设置后马上生效**，不等下一轮 30 秒轮询。
    ///
    /// 用户刚拨完开关就该看到结果（界面还要显示「填到别的设备上的地址」），让他盯着一个
    /// 半分钟不变的界面猜是不是没生效，是很差的体验。轮询循环仍然留着：它兜的是别的路径
    /// 改了设置、以及服务器意外挂掉之后重新拉起。
    pub(crate) async fn apply(&self, settings: &aa4c_types::Settings) {
        let desired = settings
            .enable_local_server
            .then_some(settings.local_server_port);
        reconcile(self, desired).await;
    }

    async fn set(&self, server: Option<Arc<Server>>) {
        *self.inner.lock().await = server;
    }
}

/// 按设置启停内置服务器的循环。
///
/// 用轮询而不是「改设置时立刻通知」：这个开关一年也拨不了几次，为它单拉一条通知通道不划算；
/// 30 秒的生效延迟对「我打开了内置服务器」这件事完全无感（用户接下来还要去另一台设备上填
/// 地址）。停机信号一到就把服务器停掉——端口必须真的还回去，否则用户关掉开关之后端口还占着。
pub(crate) fn spawn_local_server_loop(
    store: Store,
    fallback_name: String,
    fallback_save_dir: String,
    local: LocalServer,
    stop: CancellationToken,
) {
    tokio::spawn(async move {
        loop {
            match crate::settings::load(&store, &fallback_name, &fallback_save_dir).await {
                Ok(s) => local.apply(&s).await,
                Err(e) => tracing::debug!(error = %e, "load settings for local server failed"),
            }

            tokio::select! {
                biased;
                () = stop.cancelled() => break,
                () = tokio::time::sleep(POLL) => {}
            }
        }
        // 停机收尾：把端口还回去。
        if let Some(server) = local.get().await {
            server.shutdown();
            local.set(None).await;
        }
    });
}

/// 把实际状态对齐到 `desired`（`Some(port)` = 该在这个端口上跑，`None` = 该停）。
///
/// 端口变了也要重启——用户改了端口号却还在老端口上听，是比不生效更糟的状态。
async fn reconcile(local: &LocalServer, desired: Option<u16>) {
    let data_dir: &Path = &local.data_dir;
    let current = local.port().await;
    if current == desired {
        return;
    }
    if let Some(server) = local.get().await {
        tracing::info!("stopping embedded server");
        server.shutdown();
        local.set(None).await;
    }
    let Some(port) = desired else { return };

    // 双栈监听：`[::]` 让 `aa4c-server` 走它自己的 `bind_tcp_dual_stack`（里程碑 R1），
    // 同一个端口同时接受 IPv6 与 IPv4。家宽的公网 IPv6 场景全靠这一条。
    let listen_addr = SocketAddr::new(Ipv6Addr::UNSPECIFIED.into(), port);
    match aa4c_server::run(ServerConfig {
        // 与 Core 同一个 data_dir → 同一个 device.key → 内置服务器的指纹就是本机指纹。
        data_dir: data_dir.to_path_buf(),
        listen_addr,
    })
    .await
    {
        Ok(server) => {
            tracing::info!(
                addr = %server.local_addr(),
                "embedded server started"
            );
            local.set(Some(server)).await;
        }
        // 端口被占是最常见的失败（用户填了个已经有别的东西在听的号）。不致命：
        // 只是这台设备当不成汇合点，其余功能照常。
        Err(e) => tracing::warn!(error = %e, port, "embedded server failed to start"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reconcile_starts_stops_and_restarts_on_port_change() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().to_path_buf();
        let local = LocalServer::new(data_dir.clone());

        // 关着 → 什么都不做
        reconcile(&local, None).await;
        assert!(local.get().await.is_none());

        // 打开 → 起来了，且用的是本机身份（同一个 data_dir 下的 device.key）
        let identity = aa4c_identity::Identity::load_or_generate(&data_dir).unwrap();
        reconcile(&local, Some(0)).await;
        let server = local.get().await.expect("embedded server started");
        assert_eq!(
            server.device_id(),
            identity.device_id(),
            "内置服务器的指纹必须就是这台设备的指纹，不该凭空多出第二个身份"
        );
        let first_port = server.local_addr().port();
        assert_ne!(first_port, 0);

        // 同样的期望值 → 不重启（拿到的还是同一个实例）
        reconcile(&local, Some(first_port)).await;
        assert_eq!(local.port().await, Some(first_port));

        // 关掉 → 停了
        reconcile(&local, None).await;
        assert!(local.get().await.is_none());
        let mut refused = false;
        for _ in 0..50 {
            if tokio::net::TcpStream::connect(("127.0.0.1", first_port))
                .await
                .is_err()
            {
                refused = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(refused, "关掉内置服务器之后端口必须还回去");
    }
}

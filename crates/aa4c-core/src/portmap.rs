//! 自动端口映射（UPnP IGD，TRUST_DESIGN.md §6.2，V0.7 里程碑 R3）。
//!
//! 此前只有 Transmission 在给 BT 端口做映射（`port-forwarding-enabled`），**AA4C 自己的
//! 传输端口没有做**。补上之后，有公网 IPv4 的家宽也能被直连命中连接阶梯第 2 档，不必落到
//! 打洞或中继。
//!
//! ## 两层闸
//!
//! - 外层 `enable_remote`（默认**关闭**）：「不配置、不打开就完全不出网」这条默认安全的
//!   姿态由它保证。关着的时候这里一个包都不发。
//! - 内层 `enable_port_mapping`（默认**开**）：用户既然主动打开了远程连接，要的就是「能被
//!   连上」，再要求他找第二个开关才肯打洞是反的。留这个开关，是因为它确实会**在用户的
//!   路由器上开一个端口**——有人不接受这件事，界面上也写明了。
//!
//! ## 为什么只做 UPnP IGD，不做 NAT-PMP / PCP
//!
//! 不是漏了。`igd-next` 用 SSDP 组播**自己发现网关**，不需要额外知道路由器地址；而
//! NAT-PMP/PCP 的客户端库（如 `crab_nat`）一律要求调用方**自己传网关 IP**，那就得再引一个
//! 读系统路由表的平台相关依赖。收益不对称：家用路由器绝大多数支持 UPnP IGD，NAT-PMP/PCP
//! 主要是已停产的 AirPort 和部分 OpenWrt/pfSense。留作后续里程碑，不在这里堆依赖。
//!
//! ## 网络层为什么在 trait 后面
//!
//! [`PortMapper`] 的真实实现**会去改用户的路由器**。测试必须能换成假的——一个会在开发者
//! 家里路由器上真开端口的测试是不可接受的。同 `RelayDialer` / `PunchDialer` / `SidecarSpawner`
//! 的既有注入惯例（也同样刻意不引 `async-trait`，用装箱 future）。

use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use aa4c_store::Store;
use aa4c_types::Result;
use tokio_util::sync::CancellationToken;

/// 申请的租约时长。路由器可能给一个更短的值，实际续约节奏按 [`renew_after`] 算。
///
/// 取 1 小时而不是「永久」（`lease_duration = 0`）：永久映射在程序崩溃 / 断电后会**永远
/// 留在路由器上**，没人来收。带租约的映射最多留一个租约周期就自己过期，这是防止在用户
/// 路由器上留洞的最后一道保险——[`unmap_all`] 是正常路径，租约是异常路径。
const LEASE: Duration = Duration::from_secs(3600);

/// 映射在路由器管理界面里显示的名字。用户能看懂这条是谁开的，才有可能自己去删。
const DESCRIPTION: &str = "AA4C";

/// 映射失败后的重试间隔。路由器不支持 UPnP、或者用户压根不在 NAT 后面都会走到这里，
/// 属于常态而非错误，所以退避要足够长，别一直骚扰网关。
const RETRY_AFTER: Duration = Duration::from_secs(10 * 60);

/// 续约时机：租约过半就续。留一半余量是因为续约本身可能失败几次（网关重启、SSDP 丢包），
/// 还有时间重试而不至于让映射断掉。
fn renew_after(lease: Duration) -> Duration {
    // 下限 30 秒：防止路由器给了个荒唐的短租约（见过给 60 秒的）导致我们疯狂打网关。
    (lease / 2).max(Duration::from_secs(30))
}

/// 一条映射成功的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Mapped {
    /// 网关对外的公网地址。
    pub external_ip: IpAddr,
    /// 网关实际分配的外部端口——**不保证等于**我们申请的那个（申请的端口被占时会换）。
    pub external_port: u16,
    /// 网关批准的租约时长（可能短于我们申请的）。
    pub lease: Duration,
}

pub(crate) type MapFuture = Pin<Box<dyn std::future::Future<Output = Result<Mapped>> + Send>>;
pub(crate) type UnmapFuture = Pin<Box<dyn std::future::Future<Output = ()> + Send>>;

/// 端口映射后端。真实实现见 [`UpnpMapper`]，测试用假实现替换（见本模块 tests）。
pub(crate) trait PortMapper: Send + Sync + 'static {
    /// 把网关上的某个外部端口映射到本机 `local_port`。`tcp = false` 表示 UDP（QUIC 用）。
    fn map(&self, local_port: u16, tcp: bool, lease: Duration) -> MapFuture;
    /// 拆除映射。best-effort：网关已经不在了、映射已过期都不算错误，所以没有返回值。
    fn unmap(&self, external_port: u16, tcp: bool) -> UnmapFuture;
}

/// 当前生效的映射结果，供候选端点上报读取（见 `server_link::local_candidate_endpoints`）。
///
/// **不接这一步的话，映射了也没人知道，等于白做**——对端拿不到这个地址就还是只能打洞。
#[derive(Clone, Default)]
pub(crate) struct PortMapState(Arc<Mutex<Option<SocketAddr>>>);

impl PortMapState {
    /// 当前对外地址；未映射 / 已关闭时为 `None`。
    pub(crate) fn external_addr(&self) -> Option<SocketAddr> {
        *self.0.lock().expect("portmap state lock")
    }

    fn set(&self, addr: Option<SocketAddr>) {
        *self.0.lock().expect("portmap state lock") = addr;
    }
}

/// 传输端口该不该映射：两层闸都开才做（见模块文档）。
pub(crate) async fn transfer_target(
    store: &Store,
    fallback_name: &str,
    fallback_save_dir: &str,
    transfer_port: u16,
) -> Option<u16> {
    match crate::settings::load(store, fallback_name, fallback_save_dir).await {
        Ok(s) if s.enable_remote && s.enable_port_mapping => Some(transfer_port),
        Ok(_) => None,
        Err(e) => {
            tracing::debug!(error = %e, "load settings for port mapping failed");
            None
        }
    }
}

/// 内置服务器端口该不该映射（里程碑 R4）。
///
/// 闸与传输端口那条**不同**：这里看的是 `enable_local_server`，不是 `enable_remote`——
/// 内置服务器在 NAT 后面而端口没转发的话，它作为汇合点等于没用。端口取当前真正在监听的
/// 那个（服务器没起来就没什么可映射的）。
pub(crate) async fn local_server_target(
    store: &Store,
    fallback_name: &str,
    fallback_save_dir: &str,
    local: &crate::local_server::LocalServer,
) -> Option<u16> {
    match crate::settings::load(store, fallback_name, fallback_save_dir).await {
        Ok(s) if s.enable_local_server && s.enable_port_mapping => local.port().await,
        Ok(_) => None,
        Err(e) => {
            tracing::debug!(error = %e, "load settings for port mapping failed");
            None
        }
    }
}

/// 已经建立的两条映射（TCP + UDP）的外部端口，用于续约与拆除。
#[derive(Default)]
struct Active {
    /// 当前映射的**本机**端口。期望值变了（换端口 / 内置服务器重启）就得先拆再映——
    /// 还在老端口上留着映射比不映射更糟。
    local: Option<u16>,
    tcp: Option<u16>,
    udp: Option<u16>,
}

impl Active {
    fn is_empty(&self) -> bool {
        self.tcp.is_none() && self.udp.is_none()
    }
}

/// 拆掉当前全部映射并清空对外地址。**幂等**，没有映射时什么都不做。
async fn unmap_all(mapper: &Arc<dyn PortMapper>, active: &mut Active, state: &PortMapState) {
    active.local = None;
    if let Some(port) = active.tcp.take() {
        mapper.unmap(port, true).await;
    }
    if let Some(port) = active.udp.take() {
        mapper.unmap(port, false).await;
    }
    state.set(None);
}

/// 建立 TCP + UDP 两条映射；返回本轮该等多久再来（成功=续约时机，失败=退避）。
///
/// TCP 与 UDP 都要映射：AA4C 的 QUIC 与 TCP **共用同一个端口号**，只映射一个等于砍掉
/// 另一半传输能力。
async fn map_once(
    mapper: &Arc<dyn PortMapper>,
    local_port: u16,
    active: &mut Active,
    state: &PortMapState,
) -> Duration {
    let tcp = match mapper.map(local_port, true, LEASE).await {
        Ok(m) => m,
        Err(e) => {
            // 路由器不支持 UPnP、或者本机压根不在 NAT 后面——常态，不是错误。
            tracing::debug!(error = %e, "tcp port mapping unavailable");
            unmap_all(mapper, active, state).await;
            return RETRY_AFTER;
        }
    };
    active.local = Some(local_port);
    active.tcp = Some(tcp.external_port);

    let udp = match mapper.map(local_port, false, LEASE).await {
        Ok(m) => m,
        Err(e) => {
            tracing::debug!(error = %e, "udp port mapping unavailable");
            // TCP 成不了单：只有 TCP 通、QUIC 不通的半吊子状态比干脆不映射更难排查，
            // 而且会让对端拿到一个 UDP 打不通的候选，白白拖慢连接阶梯。
            unmap_all(mapper, active, state).await;
            return RETRY_AFTER;
        }
    };
    active.udp = Some(udp.external_port);

    // 两条映射的外部端口理论上应当一致（申请的是同一个），万一网关给了不同的值，
    // 以 TCP 那条为准上报——候选端点目前只表达一个 `SocketAddr`，且直连先试 TCP。
    if tcp.external_port != udp.external_port {
        tracing::warn!(
            tcp = tcp.external_port,
            udp = udp.external_port,
            "gateway assigned different external ports for tcp and udp"
        );
    }
    state.set(Some(SocketAddr::new(tcp.external_ip, tcp.external_port)));
    tracing::info!(
        external = %SocketAddr::new(tcp.external_ip, tcp.external_port),
        local_port,
        "port mapping established"
    );

    renew_after(tcp.lease.min(udp.lease))
}

/// 端口映射循环：按设置开关映射 / 拆除，租约过半续约，停机时拆掉。
///
/// 停机必须拆：留在路由器上的洞用户看不见也想不起来，而 AA4C 可能再也不启动了。租约只是
/// 兜底（崩溃 / 断电走那条路）。
///
/// `desired_port` 每轮回答一次「本轮该映哪个本机端口」（`None` = 不该映）。做成闭包是因为
/// 有两个调用方，闸与端口都不一样：传输端口看 `enable_remote` 且端口固定；内置服务器端口
/// 看 `enable_local_server`，而且端口是**动态的**（用户可以改，服务器也可能没起来）。
pub(crate) fn spawn_portmap_loop<F, Fut>(
    desired_port: F,
    mapper: Arc<dyn PortMapper>,
    state: PortMapState,
    stop: CancellationToken,
) where
    F: Fn() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Option<u16>> + Send,
{
    tokio::spawn(async move {
        let mut active = Active::default();
        loop {
            let want = desired_port().await;
            // 期望端口变了：先把老的拆掉，否则会在老端口上留一条没人用的映射。
            if active.local.is_some() && active.local != want {
                tracing::info!(
                    from = ?active.local,
                    to = ?want,
                    "port mapping target changed, removing the old one"
                );
                unmap_all(&mapper, &mut active, &state).await;
            }
            let wait = if let Some(local_port) = want {
                map_once(&mapper, local_port, &mut active, &state).await
            } else {
                // 关掉了：把已经开的洞收回去，然后按退避节奏回来看设置有没有变。
                if !active.is_empty() {
                    tracing::info!("port mapping disabled, removing existing mappings");
                    unmap_all(&mapper, &mut active, &state).await;
                }
                RETRY_AFTER
            };

            tokio::select! {
                biased;
                () = stop.cancelled() => break,
                () = tokio::time::sleep(wait) => {}
            }
        }
        // 停机收尾：不能在用户的路由器上留洞。
        unmap_all(&mapper, &mut active, &state).await;
    });
}

/// UPnP IGD 实现（`igd-next`，SSDP 自己发现网关）。
pub(crate) struct UpnpMapper {
    /// 本机 LAN 地址。UPnP 的 `AddPortMapping` 要求指明「转发到哪台机器」，不能填通配。
    local_ip: IpAddr,
}

impl UpnpMapper {
    /// `local_ip` 取「本机出网用哪个 IP」的探测结果；探不到就没法映射，返回 `None`。
    pub(crate) fn new(local_ip: Option<IpAddr>) -> Option<Self> {
        local_ip.map(|local_ip| Self { local_ip })
    }
}

impl PortMapper for UpnpMapper {
    fn map(&self, local_port: u16, tcp: bool, lease: Duration) -> MapFuture {
        let local_ip = self.local_ip;
        Box::pin(async move {
            use igd_next::PortMappingProtocol;

            let protocol = if tcp {
                PortMappingProtocol::TCP
            } else {
                PortMappingProtocol::UDP
            };
            let gateway = igd_next::aio::tokio::search_gateway(Default::default())
                .await
                .map_err(|e| net_err(format!("no upnp gateway: {e}")))?;
            let external_ip = gateway
                .get_external_ip()
                .await
                .map_err(|e| net_err(format!("upnp external ip: {e}")))?;

            let local = SocketAddr::new(local_ip, local_port);
            let lease_secs = u32::try_from(lease.as_secs()).unwrap_or(u32::MAX);

            // 先试「外部端口 = 本机端口」：这样对端看到的端口和本机配置的一致，用户排查
            // 起来也直观。被别的设备占了才退而求其次让网关随便挑一个。
            match gateway
                .add_port(protocol, local_port, local, lease_secs, DESCRIPTION)
                .await
            {
                Ok(()) => Ok(Mapped {
                    external_ip,
                    external_port: local_port,
                    lease,
                }),
                Err(e) => {
                    tracing::debug!(error = %e, "preferred external port unavailable, asking gateway to pick");
                    let external_port = gateway
                        .add_any_port(protocol, local, lease_secs, DESCRIPTION)
                        .await
                        .map_err(|e| net_err(format!("upnp add port: {e}")))?;
                    Ok(Mapped {
                        external_ip,
                        external_port,
                        lease,
                    })
                }
            }
        })
    }

    fn unmap(&self, external_port: u16, tcp: bool) -> UnmapFuture {
        Box::pin(async move {
            use igd_next::PortMappingProtocol;

            let protocol = if tcp {
                PortMappingProtocol::TCP
            } else {
                PortMappingProtocol::UDP
            };
            // 全程 best-effort：网关没了、映射早过期了都无所谓，本来就是要它消失。
            match igd_next::aio::tokio::search_gateway(Default::default()).await {
                Ok(gateway) => {
                    if let Err(e) = gateway.remove_port(protocol, external_port).await {
                        tracing::debug!(error = %e, external_port, "upnp remove port failed");
                    }
                }
                Err(e) => tracing::debug!(error = %e, "no upnp gateway while removing mapping"),
            }
        })
    }
}

fn net_err(msg: String) -> aa4c_types::Aa4cError {
    aa4c_types::Aa4cError::Network(msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// 假后端。**测试一律走这里**：真实现会去改用户的路由器，跑一次就在上面开个真端口。
    struct FakeMapper {
        mapped: Mutex<Vec<(u16, bool)>>,
        unmapped: Mutex<Vec<(u16, bool)>>,
        calls: AtomicUsize,
        fail: bool,
    }

    impl FakeMapper {
        fn new(fail: bool) -> Arc<Self> {
            Arc::new(Self {
                mapped: Mutex::new(Vec::new()),
                unmapped: Mutex::new(Vec::new()),
                calls: AtomicUsize::new(0),
                fail,
            })
        }
    }

    impl PortMapper for FakeMapper {
        fn map(&self, local_port: u16, tcp: bool, lease: Duration) -> MapFuture {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Box::pin(async { Err(net_err("no gateway".into())) });
            }
            self.mapped.lock().unwrap().push((local_port, tcp));
            Box::pin(async move {
                Ok(Mapped {
                    external_ip: "203.0.113.7".parse().unwrap(),
                    external_port: local_port,
                    lease,
                })
            })
        }

        fn unmap(&self, external_port: u16, tcp: bool) -> UnmapFuture {
            self.unmapped.lock().unwrap().push((external_port, tcp));
            Box::pin(async {})
        }
    }

    #[tokio::test]
    async fn maps_both_tcp_and_udp_and_publishes_the_external_address() {
        let mapper = FakeMapper::new(false);
        let dyn_mapper: Arc<dyn PortMapper> = mapper.clone();
        let state = PortMapState::default();
        let mut active = Active::default();

        let wait = map_once(&dyn_mapper, 42420, &mut active, &state).await;

        // QUIC 与 TCP 共用同一个端口号，只映射一条等于砍掉一半传输能力
        assert_eq!(
            *mapper.mapped.lock().unwrap(),
            vec![(42420, true), (42420, false)],
            "TCP 和 UDP 都要映射"
        );
        assert_eq!(
            state.external_addr(),
            Some("203.0.113.7:42420".parse().unwrap()),
            "外部地址必须发布出去，否则映射了也没人知道"
        );
        assert_eq!(wait, renew_after(LEASE), "成功后应当按续约节奏回来");
    }

    #[tokio::test]
    async fn a_failed_udp_mapping_rolls_the_tcp_one_back() {
        // 只有 TCP 通、QUIC 不通的半吊子状态比干脆不映射更难排查，还会让对端拿到一个
        // UDP 打不通的候选。所以 UDP 失败要把 TCP 那条也收回去。
        struct UdpFails(Mutex<Vec<(u16, bool)>>);
        impl PortMapper for UdpFails {
            fn map(&self, local_port: u16, tcp: bool, lease: Duration) -> MapFuture {
                if tcp {
                    Box::pin(async move {
                        Ok(Mapped {
                            external_ip: "203.0.113.7".parse().unwrap(),
                            external_port: local_port,
                            lease,
                        })
                    })
                } else {
                    Box::pin(async { Err(net_err("udp blocked".into())) })
                }
            }
            fn unmap(&self, external_port: u16, tcp: bool) -> UnmapFuture {
                self.0.lock().unwrap().push((external_port, tcp));
                Box::pin(async {})
            }
        }

        let mapper = Arc::new(UdpFails(Mutex::new(Vec::new())));
        let dyn_mapper: Arc<dyn PortMapper> = mapper.clone();
        let state = PortMapState::default();
        let mut active = Active::default();

        let wait = map_once(&dyn_mapper, 42420, &mut active, &state).await;

        assert_eq!(
            *mapper.0.lock().unwrap(),
            vec![(42420, true)],
            "UDP 失败后要把已经建好的 TCP 映射撤回"
        );
        assert_eq!(state.external_addr(), None, "半吊子状态不该对外上报");
        assert_eq!(wait, RETRY_AFTER);
        assert!(active.is_empty());
    }

    #[tokio::test]
    async fn a_failed_mapping_backs_off_and_publishes_nothing() {
        let mapper = FakeMapper::new(true);
        let dyn_mapper: Arc<dyn PortMapper> = mapper.clone();
        let state = PortMapState::default();
        let mut active = Active::default();

        let wait = map_once(&dyn_mapper, 42420, &mut active, &state).await;

        assert_eq!(wait, RETRY_AFTER, "路由器不支持 UPnP 是常态，退避要够长");
        assert_eq!(state.external_addr(), None);
        assert_eq!(
            mapper.calls.load(Ordering::SeqCst),
            1,
            "TCP 就失败了，不该再试 UDP"
        );
    }

    #[tokio::test]
    async fn unmap_all_is_idempotent_and_clears_the_published_address() {
        let mapper = FakeMapper::new(false);
        let dyn_mapper: Arc<dyn PortMapper> = mapper.clone();
        let state = PortMapState::default();
        let mut active = Active::default();
        map_once(&dyn_mapper, 42420, &mut active, &state).await;

        unmap_all(&dyn_mapper, &mut active, &state).await;
        assert_eq!(
            *mapper.unmapped.lock().unwrap(),
            vec![(42420, true), (42420, false)]
        );
        assert_eq!(state.external_addr(), None);

        // 再拆一次：不该重复发拆除请求
        unmap_all(&dyn_mapper, &mut active, &state).await;
        assert_eq!(mapper.unmapped.lock().unwrap().len(), 2, "幂等");
    }

    #[tokio::test]
    async fn changing_the_target_port_removes_the_old_mapping_first() {
        // 内置服务器换了端口（或者用户改了端口号）：还在老端口上留着映射，比不映射更糟——
        // 路由器上多一条没人用的转发，对端还可能照那个地址白连一场。
        let mapper = FakeMapper::new(false);
        let dyn_mapper: Arc<dyn PortMapper> = mapper.clone();
        let state = PortMapState::default();
        let mut active = Active::default();

        map_once(&dyn_mapper, 42421, &mut active, &state).await;
        assert_eq!(active.local, Some(42421));

        // 模拟循环里那一步：期望值变了就先拆
        unmap_all(&dyn_mapper, &mut active, &state).await;
        map_once(&dyn_mapper, 42999, &mut active, &state).await;

        assert_eq!(
            *mapper.unmapped.lock().unwrap(),
            vec![(42421, true), (42421, false)],
            "换端口前必须先拆掉老那条"
        );
        assert_eq!(active.local, Some(42999));
        assert_eq!(
            state.external_addr(),
            Some("203.0.113.7:42999".parse().unwrap())
        );
    }

    #[test]
    fn renewal_happens_at_half_the_lease_but_never_too_often() {
        assert_eq!(
            renew_after(Duration::from_secs(3600)),
            Duration::from_secs(1800)
        );
        // 见过给 60 秒租约的网关：照 lease/2 算就是每 30 秒打一次，太凶
        assert_eq!(
            renew_after(Duration::from_secs(60)),
            Duration::from_secs(30)
        );
        assert_eq!(
            renew_after(Duration::from_secs(10)),
            Duration::from_secs(30)
        );
    }
}

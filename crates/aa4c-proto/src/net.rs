//! 双栈监听与地址规范化（TRUST_DESIGN.md §6.1，V0.7 里程碑 R1）。
//!
//! 此前全链路只绑 `0.0.0.0`，IPv6 根本不会被选中。而国内家宽普遍下发**公网 IPv6**、
//! IPv4 反而在运营商级 NAT（CGNAT，`100.64.0.0/10`）后面——打通之后大量场景可以直接
//! 落到连接阶梯第 2 档「公网直连」，跳过打洞与中继。
//!
//! 要说清边界：**IPv6 替掉的是打洞和中继，不是汇合点**（TRUST_DESIGN.md §3.2）。
//! 对端地址仍然需要被发现，这一点不因为有公网 IPv6 而改变。
//!
//! 放在 `aa4c-proto` 而不是 `aa4c-types`：`aa4c-transfer` 与 `aa4c-server` 都要用它，
//! 而 proto 本来就承担传输层管道（`read_message`/`write_message`），types 是纯类型。

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, UdpSocket};

use socket2::{Domain, Protocol, Socket, Type};

/// TCP `listen()` 的 backlog。取一个宽松值：AA4C 的入站连接是突发式的（一次同步会开
/// 好几条），backlog 太小会在高并发时直接丢 SYN。
const BACKLOG: i32 = 128;

/// 绑一个**双栈** TCP 监听 socket：同时接受 IPv6 与（以 IPv4 映射地址呈现的）IPv4。
///
/// IPv6 不可用时**回落到纯 IPv4**，行为与打通双栈之前完全一致——这是不回归的保证：
/// 有些容器/内核编译选项下根本没有 IPv6，不能因此起不来。
pub fn bind_tcp_dual_stack(port: u16) -> io::Result<TcpListener> {
    match try_bind_tcp_v6(port) {
        Ok(l) => Ok(l),
        Err(e) => {
            tracing_v6_fallback("tcp", &e);
            TcpListener::bind((Ipv4Addr::UNSPECIFIED, port))
        }
    }
}

/// 绑一个**双栈** UDP socket（QUIC 用；交给 `quinn::Endpoint::new` 接管）。
/// 同 [`bind_tcp_dual_stack`]，IPv6 不可用时回落纯 IPv4。
pub fn bind_udp_dual_stack(port: u16) -> io::Result<UdpSocket> {
    match try_bind_udp_v6(port) {
        Ok(s) => Ok(s),
        Err(e) => {
            tracing_v6_fallback("udp", &e);
            UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port))
        }
    }
}

fn try_bind_tcp_v6(port: u16) -> io::Result<TcpListener> {
    let sock = Socket::new(Domain::IPV6, Type::STREAM, Some(Protocol::TCP))?;
    prepare_v6(&sock)?;
    // 与 `std`/`tokio` 的既有行为对齐：它们默认就开 SO_REUSEADDR，不开会让重启时
    // 撞上 TIME_WAIT 的端口绑不上。
    sock.set_reuse_address(true)?;
    sock.bind(&SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port).into())?;
    sock.listen(BACKLOG)?;
    let listener: TcpListener = sock.into();
    // tokio 接管前必须是非阻塞的。
    listener.set_nonblocking(true)?;
    Ok(listener)
}

fn try_bind_udp_v6(port: u16) -> io::Result<UdpSocket> {
    let sock = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
    prepare_v6(&sock)?;
    sock.bind(&SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port).into())?;
    let socket: UdpSocket = sock.into();
    socket.set_nonblocking(true)?;
    Ok(socket)
}

/// **必须显式**关掉 `IPV6_V6ONLY`，不能依赖平台默认值：
/// Windows 默认是**开**（那样 IPv4 会整个失联）；Linux 取决于 `net.ipv6.bindv6only`
/// 这个 sysctl，同样不该赌它；OpenBSD 则**根本不允许关**，那里这一步会失败，调用方
/// 因此回落纯 IPv4（见 [`bind_tcp_dual_stack`]）。
fn prepare_v6(sock: &Socket) -> io::Result<()> {
    sock.set_only_v6(false)
}

fn tracing_v6_fallback(kind: &str, e: &io::Error) {
    tracing::debug!(
        transport = kind,
        error = %e,
        "dual-stack bind unavailable, falling back to IPv4-only"
    );
}

/// 把 IPv4 映射地址（`::ffff:a.b.c.d`）还原成普通 IPv4。
///
/// 双栈监听之后，一条**普通 IPv4 入站连接**的 `peer_addr()` 会以映射形式出现。它虽然
/// 功能上等价（连得回去、`SocketAddr` 也能正常解析），但会一路写进 `devices.last_addr`
/// 这类**落库的文本**里，让同一台设备在打通双栈前后存出两种写法，界面上也会露出
/// `::ffff:` 这种没人看得懂的前缀。在拿到地址的第一时间还原掉，是最省事的做法。
///
/// 真正的 IPv6 地址原样返回。
#[must_use]
pub fn normalize_mapped(addr: SocketAddr) -> SocketAddr {
    match addr {
        SocketAddr::V6(v6) => match v6.ip().to_ipv4_mapped() {
            Some(v4) => SocketAddr::new(IpAddr::V4(v4), v6.port()),
            None => addr,
        },
        v4 => v4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapped_ipv4_is_restored_to_plain_ipv4() {
        let mapped: SocketAddr = "[::ffff:192.168.1.5]:42420".parse().unwrap();
        assert_eq!(
            normalize_mapped(mapped),
            "192.168.1.5:42420".parse().unwrap()
        );
    }

    #[test]
    fn real_ipv6_and_ipv4_are_left_alone() {
        // 真 IPv6 不能被动到
        let v6: SocketAddr = "[2408:8000::1]:42420".parse().unwrap();
        assert_eq!(normalize_mapped(v6), v6);
        // 回环 IPv6 也是真 IPv6，不是映射地址
        let loop6: SocketAddr = "[::1]:42420".parse().unwrap();
        assert_eq!(normalize_mapped(loop6), loop6);
        let v4: SocketAddr = "192.168.1.5:42420".parse().unwrap();
        assert_eq!(normalize_mapped(v4), v4);
    }

    #[test]
    fn dual_stack_tcp_accepts_ipv4_connections() {
        let listener = bind_tcp_dual_stack(0).expect("bind");
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(false).unwrap();

        // 关键断言：绑的是 `[::]`，但**普通 IPv4 客户端必须连得上**——这正是
        // `IPV6_V6ONLY = false` 要保证的事（不显式设置时 Windows 上会连不上）。
        let handle = std::thread::spawn(move || listener.accept().map(|(_, peer)| peer));
        std::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port)).expect("ipv4 connect");
        let peer = handle.join().unwrap().expect("accept");

        // 而且拿到的地址应当已经还原成普通 IPv4，不是 `::ffff:127.0.0.1`
        assert!(
            normalize_mapped(peer).is_ipv4(),
            "IPv4 客户端的地址应还原为 IPv4，实际 {peer}"
        );
    }

    #[test]
    fn dual_stack_udp_binds_and_reports_v6_local_addr() {
        let sock = bind_udp_dual_stack(0).expect("bind");
        let local = sock.local_addr().unwrap();
        // 双栈成功时本地地址是 `[::]`；若环境没有 IPv6 则回落 IPv4，两种都算通过
        // （这条用例守的是"不 panic、拿得到端口"，不是"这台机器一定有 IPv6"）。
        assert_ne!(local.port(), 0);
    }
}

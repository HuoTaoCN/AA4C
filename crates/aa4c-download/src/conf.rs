//! 启动配置生成：端口探测 + 密钥生成 + conf 文件（DOWNLOAD_DESIGN.md §3.1）。
//!
//! 全部选项（含密钥）写进一个每次启动重新生成的 conf 文件，命令行只传
//! `--conf-path`——密钥不放命令行是硬要求（命令行参数对本机任意用户的进程经
//! `ps`/WMI 可见）。

use std::path::{Path, PathBuf};

use aa4c_types::{Aa4cError, Result};

use crate::util::{generate_secret, probe_free_port};

/// 本次启动实际生效的配置（供 `Aria2Client` 连接与健康检查使用）。
pub struct AriaConf {
    pub port: u16,
    pub secret: String,
    pub conf_path: PathBuf,
}

/// 单文件最大连接数（分段下载加速）未设置时的兜底值——**不是** aria2 自己的默认值
/// （aria2 默认 `-x 1`，等同不开加速）。见 `aa4c_types::Settings::download_max_connections_per_file`
/// 文档：这是"对标 FDM/IDM 多线程下载"的核心改动，开箱即用就要比单连接快，不能
/// 沿用 aria2 保守的默认值。
const DEFAULT_MAX_CONNECTIONS_PER_FILE: u32 = 5;
/// aria2 `-x`/`--max-connection-per-server` 的官方上限。
const MAX_CONNECTIONS_PER_FILE_CAP: u32 = 16;
/// 小于这个大小的文件不分段——避免几十 KB 的小文件也开好几个连接，得不偿失。
const MIN_SPLIT_SIZE: &str = "5M";
/// 未设置 User-Agent 时的内置兜底——**不是** aria2 自己的默认值。aria2 默认发
/// `aria2/1.37.0`，相当一部分站点见到就直接 403 / 跳验证页，用户只会看到一条
/// 没头没脑的下载失败。同 Motrix 的取舍（它也把 Chrome UA 设成默认值）。
const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";

/// aria2 引擎级可选项（对标 Motrix 的 system config，DOWNLOAD_DESIGN.md §9）。
/// 打包成结构体而不是继续往 `write_conf` 上堆平行参数——参数已经到 6 个，
/// 再加就必然出现"调用点传错位置"的隐患（同 `DownloadLimits` 的既有取舍）。
#[derive(Debug, Clone, Default)]
pub(crate) struct AriaOptions {
    pub speed_limit_kbps: Option<u32>,
    pub upload_limit_kbps: Option<u32>,
    pub concurrency: Option<u32>,
    pub max_connections_per_file: Option<u32>,
    pub user_agent: Option<String>,
    pub proxy: Option<String>,
    pub proxy_bypass: Option<String>,
}

/// 生成本次启动用的 conf 文件（`<data_dir>/aria2.conf`，Unix 上 0600 权限）。
/// `session_path` 已存在时才写 `input-file`（首次启动没有历史 session）。
/// `speed_limit_kbps`/`concurrency` 是 D3 新增的可选限速/并发设置
/// （`None`/`0` 不写对应行，走 aria2 自己的默认行为，DOWNLOAD_DESIGN.md §9）。
/// `opts.max_connections_per_file` / `opts.user_agent` 语义不同——`None` 时落到
/// 内置兜底值而不是 aria2 自己的默认值，见上面两个常量的文档。
pub(crate) fn write_conf(
    data_dir: &Path,
    download_dir: &Path,
    host_pid: u32,
    opts: &AriaOptions,
) -> Result<AriaConf> {
    std::fs::create_dir_all(data_dir).map_err(Aa4cError::Io)?;
    let port = probe_free_port()?;
    let secret = generate_secret();
    let conf_path = data_dir.join("aria2.conf");
    let session_path = data_dir.join("aria2.session");

    let mut body = String::new();
    body.push_str("enable-rpc=true\n");
    body.push_str(&format!("rpc-listen-port={port}\n"));
    body.push_str("rpc-listen-all=false\n");
    body.push_str(&format!("rpc-secret={secret}\n"));
    body.push_str(&format!("dir={}\n", download_dir.display()));
    // 宿主（AA4C）消失时 aria2c 自行退出——不留孤儿进程，不需要 PID 文件簿记
    // （aria2 手册明确这个选项就是为"被父进程 fork 出来"的场景设计的）。
    body.push_str(&format!("stop-with-process={host_pid}\n"));
    body.push_str(&format!("save-session={}\n", session_path.display()));
    if session_path.exists() {
        body.push_str(&format!("input-file={}\n", session_path.display()));
    }
    body.push_str("save-session-interval=30\n");
    body.push_str("continue=true\n");
    if let Some(limit) = opts.speed_limit_kbps.filter(|&n| n > 0) {
        body.push_str(&format!("max-overall-download-limit={limit}K\n"));
    }
    if let Some(limit) = opts.upload_limit_kbps.filter(|&n| n > 0) {
        body.push_str(&format!("max-overall-upload-limit={limit}K\n"));
    }
    if let Some(n) = opts.concurrency.filter(|&n| n > 0) {
        body.push_str(&format!("max-concurrent-downloads={n}\n"));
    }
    // 分段下载加速（对标 FDM/IDM 的"多线程下载"）：无条件写入，不是"设置了才写"——
    // 未设置时用 DEFAULT_MAX_CONNECTIONS_PER_FILE 兜底，而不是 aria2 保守的默认值 1
    // （见上面常量文档）。`split` 跟连接数保持一致才有意义（分段数 < 连接数时连接
    // 数用不满）。
    let conns = opts
        .max_connections_per_file
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_MAX_CONNECTIONS_PER_FILE)
        .min(MAX_CONNECTIONS_PER_FILE_CAP);
    body.push_str(&format!("max-connection-per-server={conns}\n"));
    body.push_str(&format!("split={conns}\n"));
    body.push_str(&format!("min-split-size={MIN_SPLIT_SIZE}\n"));
    // 同 max-connection-per-server：无条件写入，未设置时用浏览器 UA 兜底而不是
    // aria2 自己那个会被站点拒的默认值（见 DEFAULT_USER_AGENT 文档）。
    let ua = opts
        .user_agent
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_USER_AGENT);
    body.push_str(&format!("user-agent={ua}\n"));
    // 代理：没配就一行都不写，走 aria2 的"不使用代理"默认行为。
    if let Some(proxy) = opts
        .proxy
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        body.push_str(&format!("all-proxy={proxy}\n"));
        if let Some(bypass) = opts
            .proxy_bypass
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            body.push_str(&format!("no-proxy={bypass}\n"));
        }
    }

    write_file_0600(&conf_path, &body)?;

    Ok(AriaConf {
        port,
        secret,
        conf_path,
    })
}

#[cfg(unix)]
fn write_file_0600(path: &Path, body: &str) -> Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(Aa4cError::Io)?;
    f.write_all(body.as_bytes()).map_err(Aa4cError::Io)?;
    Ok(())
}

#[cfg(not(unix))]
fn write_file_0600(path: &Path, body: &str) -> Result<()> {
    std::fs::write(path, body).map_err(Aa4cError::Io)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 只关心"单文件最大连接数"这一项的用例的简写。
    fn conns(n: Option<u32>) -> AriaOptions {
        AriaOptions {
            max_connections_per_file: n,
            ..AriaOptions::default()
        }
    }

    #[test]
    fn generates_distinct_ports_and_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let a = write_conf(dir.path(), dir.path(), 1, &AriaOptions::default()).unwrap();
        std::fs::remove_file(&a.conf_path).unwrap();
        let b = write_conf(dir.path(), dir.path(), 1, &AriaOptions::default()).unwrap();
        assert_ne!(a.secret, b.secret);
        // 端口理论上可能撞（极小概率），但密钥绝不会撞
        assert!(a.secret.len() >= 32);
    }

    #[test]
    fn conf_contains_expected_directives_and_no_input_file_on_first_boot() {
        let dir = tempfile::tempdir().unwrap();
        let conf = write_conf(dir.path(), dir.path(), 42, &AriaOptions::default()).unwrap();
        let body = std::fs::read_to_string(&conf.conf_path).unwrap();
        assert!(body.contains("enable-rpc=true"));
        assert!(body.contains("rpc-listen-all=false"));
        assert!(body.contains(&format!("rpc-secret={}", conf.secret)));
        assert!(body.contains("stop-with-process=42"));
        assert!(!body.contains("input-file="));
        // 未设限时不写这两行，走 aria2 自己的默认行为
        assert!(!body.contains("max-overall-download-limit"));
        assert!(!body.contains("max-concurrent-downloads"));
        // 分段下载加速无条件写入，未设置时落到 DEFAULT_MAX_CONNECTIONS_PER_FILE（5），
        // 不是 aria2 自己的默认值 1
        assert!(body.contains("max-connection-per-server=5"));
        assert!(body.contains("split=5"));
        assert!(body.contains("min-split-size=5M"));
        // User-Agent 同理无条件写入，未设置时落到内置浏览器 UA，不是 aria2 那个会被
        // 站点拒的默认值
        assert!(body.contains("user-agent=Mozilla/5.0"));
        // 上传限速/代理没配就一行都不写
        assert!(!body.contains("max-overall-upload-limit"));
        assert!(!body.contains("all-proxy"));
        assert!(!body.contains("no-proxy"));
    }

    #[test]
    fn conf_writes_upload_limit_user_agent_and_proxy_when_set() {
        let dir = tempfile::tempdir().unwrap();
        let conf = write_conf(
            dir.path(),
            dir.path(),
            1,
            &AriaOptions {
                upload_limit_kbps: Some(256),
                user_agent: Some("MyAgent/1.0".into()),
                proxy: Some("http://127.0.0.1:8080".into()),
                proxy_bypass: Some("localhost,192.168.0.0/16".into()),
                ..AriaOptions::default()
            },
        )
        .unwrap();
        let body = std::fs::read_to_string(&conf.conf_path).unwrap();
        assert!(body.contains("max-overall-upload-limit=256K"));
        assert!(body.contains("user-agent=MyAgent/1.0"));
        assert!(body.contains("all-proxy=http://127.0.0.1:8080"));
        assert!(body.contains("no-proxy=localhost,192.168.0.0/16"));
    }

    #[test]
    fn conf_falls_back_to_builtin_user_agent_when_blank() {
        // 用户把输入框清空成几个空格 ≠ "我要一个空 UA"——空白按未设置处理
        let dir = tempfile::tempdir().unwrap();
        let conf = write_conf(
            dir.path(),
            dir.path(),
            1,
            &AriaOptions {
                user_agent: Some("   ".into()),
                ..AriaOptions::default()
            },
        )
        .unwrap();
        let body = std::fs::read_to_string(&conf.conf_path).unwrap();
        assert!(body.contains("user-agent=Mozilla/5.0"));
    }

    #[test]
    fn conf_omits_proxy_bypass_when_proxy_not_set() {
        // 没有代理时 no-proxy 毫无意义，不该单独写进去
        let dir = tempfile::tempdir().unwrap();
        let conf = write_conf(
            dir.path(),
            dir.path(),
            1,
            &AriaOptions {
                proxy_bypass: Some("localhost".into()),
                ..AriaOptions::default()
            },
        )
        .unwrap();
        let body = std::fs::read_to_string(&conf.conf_path).unwrap();
        assert!(!body.contains("no-proxy"));
    }

    #[test]
    fn conf_uses_configured_max_connections_per_file() {
        let dir = tempfile::tempdir().unwrap();
        let conf = write_conf(dir.path(), dir.path(), 1, &conns(Some(10))).unwrap();
        let body = std::fs::read_to_string(&conf.conf_path).unwrap();
        assert!(body.contains("max-connection-per-server=10"));
        assert!(body.contains("split=10"));
    }

    #[test]
    fn conf_clamps_max_connections_per_file_to_aria2_cap() {
        let dir = tempfile::tempdir().unwrap();
        let conf = write_conf(dir.path(), dir.path(), 1, &conns(Some(999))).unwrap();
        let body = std::fs::read_to_string(&conf.conf_path).unwrap();
        assert!(body.contains("max-connection-per-server=16"));
        assert!(body.contains("split=16"));
    }

    #[test]
    fn conf_falls_back_to_default_when_zero() {
        let dir = tempfile::tempdir().unwrap();
        let conf = write_conf(dir.path(), dir.path(), 1, &conns(Some(0))).unwrap();
        let body = std::fs::read_to_string(&conf.conf_path).unwrap();
        assert!(body.contains("max-connection-per-server=5"));
    }

    #[test]
    fn conf_includes_input_file_when_session_exists() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("aria2.session"), "").unwrap();
        let conf = write_conf(dir.path(), dir.path(), 1, &AriaOptions::default()).unwrap();
        let body = std::fs::read_to_string(&conf.conf_path).unwrap();
        assert!(body.contains("input-file="));
    }

    #[test]
    fn conf_includes_speed_limit_and_concurrency_when_set() {
        let dir = tempfile::tempdir().unwrap();
        let conf = write_conf(
            dir.path(),
            dir.path(),
            1,
            &AriaOptions {
                speed_limit_kbps: Some(500),
                concurrency: Some(3),
                max_connections_per_file: Some(8),
                ..AriaOptions::default()
            },
        )
        .unwrap();
        let body = std::fs::read_to_string(&conf.conf_path).unwrap();
        assert!(body.contains("max-overall-download-limit=500K"));
        assert!(body.contains("max-concurrent-downloads=3"));
    }

    #[test]
    fn conf_omits_speed_limit_when_zero() {
        let dir = tempfile::tempdir().unwrap();
        let conf = write_conf(
            dir.path(),
            dir.path(),
            1,
            &AriaOptions {
                speed_limit_kbps: Some(0),
                ..AriaOptions::default()
            },
        )
        .unwrap();
        let body = std::fs::read_to_string(&conf.conf_path).unwrap();
        assert!(!body.contains("max-overall-download-limit"));
    }

    #[cfg(unix)]
    #[test]
    fn conf_file_is_0600_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let conf = write_conf(dir.path(), dir.path(), 1, &AriaOptions::default()).unwrap();
        let mode = std::fs::metadata(&conf.conf_path)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

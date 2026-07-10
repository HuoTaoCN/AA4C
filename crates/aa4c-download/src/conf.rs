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

/// 生成本次启动用的 conf 文件（`<data_dir>/aria2.conf`，Unix 上 0600 权限）。
/// `session_path` 已存在时才写 `input-file`（首次启动没有历史 session）。
pub(crate) fn write_conf(data_dir: &Path, download_dir: &Path, host_pid: u32) -> Result<AriaConf> {
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

    #[test]
    fn generates_distinct_ports_and_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let a = write_conf(dir.path(), dir.path(), 1).unwrap();
        std::fs::remove_file(&a.conf_path).unwrap();
        let b = write_conf(dir.path(), dir.path(), 1).unwrap();
        assert_ne!(a.secret, b.secret);
        // 端口理论上可能撞（极小概率），但密钥绝不会撞
        assert!(a.secret.len() >= 32);
    }

    #[test]
    fn conf_contains_expected_directives_and_no_input_file_on_first_boot() {
        let dir = tempfile::tempdir().unwrap();
        let conf = write_conf(dir.path(), dir.path(), 42).unwrap();
        let body = std::fs::read_to_string(&conf.conf_path).unwrap();
        assert!(body.contains("enable-rpc=true"));
        assert!(body.contains("rpc-listen-all=false"));
        assert!(body.contains(&format!("rpc-secret={}", conf.secret)));
        assert!(body.contains("stop-with-process=42"));
        assert!(!body.contains("input-file="));
    }

    #[test]
    fn conf_includes_input_file_when_session_exists() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("aria2.session"), "").unwrap();
        let conf = write_conf(dir.path(), dir.path(), 1).unwrap();
        let body = std::fs::read_to_string(&conf.conf_path).unwrap();
        assert!(body.contains("input-file="));
    }

    #[cfg(unix)]
    #[test]
    fn conf_file_is_0600_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let conf = write_conf(dir.path(), dir.path(), 1).unwrap();
        let mode = std::fs::metadata(&conf.conf_path)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

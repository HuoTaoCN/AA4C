//! Transmission 启动配置生成（DOWNLOAD_DESIGN.md §3.6.2）：`settings.json` 每次
//! 启动整体重写，命令行只传 `-f`（前台模式，硬要求——不传的话 transmission-daemon
//! 默认 fork 到后台、父进程立即退出，`SidecarSpawner` 拿到的句柄会抓错进程）+
//! `--config-dir=<path>`。凭据（RPC 用户名/密码）同 aria2 密钥一样不进命令行，
//! 全部写进这个每次启动重写的文件。

use std::path::{Path, PathBuf};

use aa4c_types::{Aa4cError, Result};
use serde_json::json;

use crate::util::{generate_secret, probe_free_port};

/// 本次启动实际生效的配置（供 `TransmissionClient` 连接使用，D2.3）。
pub struct TransmissionConf {
    pub port: u16,
    pub username: String,
    pub password: String,
    pub config_dir: PathBuf,
}

/// 生成本次启动用的 `<config_dir>/settings.json`。`config_dir` 本身（不是
/// `data_dir` 下某个子目录名固定值，由调用方决定）承担了 aria2 `aria2.conf`
/// 同样的角色——每次启动整体覆盖，不做增量修改。
///
/// `speed_limit_kbps`/`concurrency`/`ratio_limit`/`idle_seeding_limit_minutes`
/// 是 D3 新增的可选限速/并发/分享率/空闲做种超时设置（`None` 不写对应键，走
/// Transmission 自己的默认行为）。**键名均为本机真机验证过的配置文件格式**
/// （起真实 `transmission-daemon` + `transmission-remote --session-info` 核实，
/// 不是照抄 RPC session-set 的参数名——`ratio-limit`/`ratio-limit-enabled` 与
/// 官方 RPC spec 文档写的 `seedRatioLimit`/`seedRatioLimited` 不是同一个名字，
/// 后者在配置文件里完全不生效，见 DOWNLOAD_DESIGN.md §9）。
pub(crate) fn write_settings(
    config_dir: &Path,
    download_dir: &Path,
    speed_limit_kbps: Option<u32>,
    concurrency: Option<u32>,
    ratio_limit: Option<f64>,
    idle_seeding_limit_minutes: Option<u32>,
) -> Result<TransmissionConf> {
    std::fs::create_dir_all(config_dir).map_err(Aa4cError::Io)?;
    let port = probe_free_port()?;
    let username = generate_secret();
    let password = generate_secret();

    let mut body = json!({
        "rpc-enabled": true,
        "rpc-bind-address": "127.0.0.1",
        "rpc-port": port,
        "rpc-authentication-required": true,
        "rpc-username": username,
        "rpc-password": password,
        "rpc-whitelist-enabled": false,
        "download-dir": download_dir.display().to_string(),
        "dht-enabled": true,
        "pex-enabled": true,
        "lpd-enabled": true,
        "port-forwarding-enabled": true,
        "encryption": 1,
    });
    let obj = body
        .as_object_mut()
        .expect("settings body is a JSON object");
    if let Some(limit) = speed_limit_kbps.filter(|&n| n > 0) {
        obj.insert("speed-limit-down".into(), json!(limit));
        obj.insert("speed-limit-down-enabled".into(), json!(true));
    }
    if let Some(n) = concurrency.filter(|&n| n > 0) {
        obj.insert("download-queue-size".into(), json!(n));
        obj.insert("download-queue-enabled".into(), json!(true));
    }
    if let Some(ratio) = ratio_limit {
        obj.insert("ratio-limit".into(), json!(ratio));
        obj.insert("ratio-limit-enabled".into(), json!(true));
    }
    if let Some(minutes) = idle_seeding_limit_minutes {
        obj.insert("idle-seeding-limit".into(), json!(minutes));
        obj.insert("idle-seeding-limit-enabled".into(), json!(true));
    }

    let settings_path = config_dir.join("settings.json");
    write_file_0600(
        &settings_path,
        &serde_json::to_string_pretty(&body).unwrap(),
    )?;

    Ok(TransmissionConf {
        port,
        username,
        password,
        config_dir: config_dir.to_path_buf(),
    })
}

/// `transmission-daemon` 命令行参数：`-f`（前台模式，硬要求）+
/// `--config-dir=<path>`。收敛成固定形状，同 aria2 只传 `--conf-path` 的思路——
/// Tauri capability 的参数放行可以用精确匹配。
pub(crate) fn spawn_args(config_dir: &Path) -> Vec<String> {
    vec![
        "-f".to_string(),
        format!("--config-dir={}", config_dir.display()),
    ]
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
    fn generates_distinct_ports_and_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let a = write_settings(&dir.path().join("a"), dir.path(), None, None, None, None).unwrap();
        let b = write_settings(&dir.path().join("b"), dir.path(), None, None, None, None).unwrap();
        assert_ne!(a.username, b.username);
        assert_ne!(a.password, b.password);
        assert!(a.username.len() >= 32);
    }

    #[test]
    fn settings_contains_expected_keys() {
        let dir = tempfile::tempdir().unwrap();
        let download_dir = dir.path().join("downloads");
        let conf = write_settings(
            &dir.path().join("cfg"),
            &download_dir,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let body = std::fs::read_to_string(conf.config_dir.join("settings.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["rpc-bind-address"], "127.0.0.1");
        assert_eq!(parsed["rpc-port"], conf.port);
        assert_eq!(parsed["rpc-authentication-required"], true);
        assert_eq!(parsed["rpc-username"], conf.username);
        assert_eq!(parsed["rpc-password"], conf.password);
        assert_eq!(parsed["download-dir"], download_dir.display().to_string());
        assert_eq!(parsed["dht-enabled"], true);
        // 未设限时不写这几个键
        assert!(parsed.get("speed-limit-down").is_none());
        assert!(parsed.get("ratio-limit").is_none());
        assert!(parsed.get("idle-seeding-limit").is_none());
        assert!(parsed.get("download-queue-size").is_none());
    }

    #[test]
    fn settings_contains_limits_when_set() {
        let dir = tempfile::tempdir().unwrap();
        let conf = write_settings(
            &dir.path().join("cfg"),
            dir.path(),
            Some(500),
            Some(3),
            Some(2.0),
            Some(30),
        )
        .unwrap();
        let body = std::fs::read_to_string(conf.config_dir.join("settings.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["speed-limit-down"], 500);
        assert_eq!(parsed["speed-limit-down-enabled"], true);
        assert_eq!(parsed["download-queue-size"], 3);
        assert_eq!(parsed["download-queue-enabled"], true);
        assert_eq!(parsed["ratio-limit"], 2.0);
        assert_eq!(parsed["ratio-limit-enabled"], true);
        assert_eq!(parsed["idle-seeding-limit"], 30);
        assert_eq!(parsed["idle-seeding-limit-enabled"], true);
    }

    #[test]
    fn spawn_args_is_foreground_plus_config_dir() {
        let dir = tempfile::tempdir().unwrap();
        let args = spawn_args(dir.path());
        assert_eq!(args[0], "-f");
        assert_eq!(args[1], format!("--config-dir={}", dir.path().display()));
    }

    #[cfg(unix)]
    #[test]
    fn settings_file_is_0600_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let conf =
            write_settings(&dir.path().join("cfg"), dir.path(), None, None, None, None).unwrap();
        let mode = std::fs::metadata(conf.config_dir.join("settings.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

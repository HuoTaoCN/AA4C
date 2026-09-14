//! 下载插件的端到端测试（R1 之前住在 `aa4c-core/tests/core.rs`）。
//!
//! **仍然驱动真实 `Core`**，只是能力从类型化方法换成了 `plugin_invoke`——验的就是
//! 应用真正走的那条缝，而不是绕开它直接调 `DownloadService`。依赖方向不冲突：
//! `aa4c-core` 不依赖本 crate，本 crate 只在 dev-dependency 里用它。
//!
//! 两条都要真实外部进程（`aria2c` / `transmission-daemon`），找不到就显式 panic
//! ——那不是回归，是环境缺件，见 HANDOFF.md 的环境要求。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use aa4c_core::{Core, CoreConfig};
use aa4c_download::{DownloadPlugin, ProcessSpawner};
use aa4c_types::CoreEvent;
use serde_json::json;
use tokio::time::timeout;

/// `plugin_invoke` 统一返回 `serde_json::Value`，测试里按需还原成具体类型。
/// 这是通用调用入口的代价，写在这里比每处 `serde_json::from_value` 铺开好读。
fn from_json<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> T {
    serde_json::from_value(v).expect("plugin returned an unexpected shape")
}

/// 里程碑 D1 端到端：真实 aria2c 通过 Core 编排方法完成一次下载，`CoreEvent`
/// 正确广播，`list_downloads` 能看到落库记录。需要本机 PATH 里有 `aria2c`
/// （`brew install aria2` 等，见 HANDOFF.md 环境要求）——找不到就显式 panic。
#[tokio::test]
async fn download_end_to_end_through_core_orchestration() {
    // `tracing` 默认在 #[tokio::test] 里没有订阅者——DownloadService::start 失败时
    // 唯一的线索（spawn/健康检查失败的具体原因）是一条 tracing::warn!，平时完全
    // 看不到。这条测试驱动真实子进程，失败原因值得能看见，装一个订阅者
    // （`try_init` 幂等，多次调用/并行测试线程都安全）。
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_env_filter("aa4c_download=debug,aa4c_core=debug")
        .try_init();

    fn require_aria2c() -> PathBuf {
        let path_var = std::env::var_os("PATH").unwrap_or_default();
        let exe_name = if cfg!(windows) {
            "aria2c.exe"
        } else {
            "aria2c"
        };
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(exe_name);
            if candidate.is_file() {
                return candidate;
            }
        }
        panic!("aria2c not found in PATH — install it to run this test (see HANDOFF.md)");
    }

    async fn spawn_http_server(body: Vec<u8>) -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let body = body.clone();
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buf = vec![0u8; 4096];
                    if stream.read(&mut buf).await.unwrap_or(0) == 0 {
                        return;
                    }
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    if stream.write_all(header.as_bytes()).await.is_err() {
                        return;
                    }
                    let _ = stream.write_all(&body).await;
                });
            }
        });
        addr
    }

    let body = b"AA4C D1 Core orchestration e2e payload".repeat(200);
    let http_addr = spawn_http_server(body.clone()).await;

    let dir = tempfile::tempdir().unwrap();

    // 下载目录必须在 Core::start 之前就落库指向隔离的临时目录——DownloadService
    // 在 Core::start 内部启动时就把 download_dir 写死进 aria2 conf 文件，事后
    // update_settings 已经来不及了。不预置的话会落进 `default_download_dir()`
    // 的真实系统下载目录（这台测试机上真的发生过，测试文件混进了开发者本人的
    // ~/Downloads，见 V0.4_IMPLEMENTATION_PLAN.md D1 步骤 9 人工走查记录）。
    let download_dir = dir.path().join("test-downloads");
    {
        let seed_store = aa4c_store::Store::open(&dir.path().join("aa4c.db"))
            .await
            .unwrap();
        seed_store
            .set_setting(
                "download_dir",
                &serde_json::to_string(&download_dir.to_string_lossy().into_owned()).unwrap(),
            )
            .await
            .unwrap();
    }

    let mut config = CoreConfig::new(dir.path().to_path_buf());
    config.listen_port = 0;
    config.transfer.default_save_dir = dir.path().join("downloads");
    // mDNS 对本套件只是噪声：全部用例都用显式地址建连（见文件头注释），而每个
    // `ServiceDaemon` 自带一条 OS 线程 + 一组 5353 组播 socket，并行跑十几个用例时
    // 几十个守护进程互相挤，配对会开始超时。
    config.disable_discovery = true;
    // 端口映射的真实实现会去改**跑测试这台机器所在路由器**的配置。有几个用例会打开
    // `enable_remote`（映射的外层闸），只是碰巧在退避周期内就跑完了才没真发出去——
    // 不能靠这种巧合，直接从结构上关掉。
    config.disable_port_mapping = true;
    config.plugins.register(Arc::new(DownloadPlugin::new(
        Arc::new(ProcessSpawner::new(require_aria2c())),
        None,
    )));
    let core = Core::start(config).await.expect("core starts");

    let mut rx = core.subscribe();
    let id = core
        .plugin_invoke(
            "download",
            "add".into(),
            json!({ "url": format!("http://{http_addr}/file.bin") }),
        )
        .await
        .unwrap();

    let done_path = timeout(Duration::from_secs(20), async {
        loop {
            match rx.recv().await.unwrap() {
                CoreEvent::DownloadDone { task_id, save_path } if task_id == id => {
                    return save_path
                }
                CoreEvent::DownloadFailed { task_id, error } if task_id == id => {
                    panic!("download failed: {error}")
                }
                _ => {}
            }
        }
    })
    .await
    .expect("DownloadDone within timeout");

    assert_eq!(tokio::fs::read(&done_path).await.unwrap(), body);

    let listed = from_json::<Vec<aa4c_types::DownloadTask>>(
        core.plugin_invoke("download", "list".into(), json!({}))
            .await
            .unwrap(),
    );
    assert!(listed.iter().any(|t| t.id == id));

    core.shutdown().await.unwrap();
}

/// 里程碑 D2：真实 transmission-daemon 通过 Core 编排方法完成 magnet 添加 →
/// 落库为 `kind: Bt`（id = 40 位 infohash）→ 暂停/继续/取消全部生效，`Core::
/// add_download` 按 scheme 正确路由到 Transmission 而不是 aria2。**不测完整
/// 下载落盘**——BT 需要真实 peer/tracker 连通性，本地做种测试基础设施本身就
/// 是个不小的工程量（要另起一个 daemon 当种子端 + 一份真实 .torrent 文件 +
/// 处理 DHT/PEX 在纯回环环境下不一定能互相发现的问题），同 C5 NAT 打洞的
/// 处理先例——CI 只验证真实进程间的接线是否正确，完整下载场景靠人工走查，
/// DOWNLOAD_DESIGN.md §3.6 已经这样定。需要本机 PATH 里有 `aria2c` 与
/// `transmission-daemon`（`download_spawner` 是"本平台是否支持下载能力"的
/// 总闸，即使这条测试本身不碰 aria2 也得配，见 `CoreConfig` 文档）。
#[tokio::test]
async fn bt_download_routes_through_core_orchestration() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_env_filter("aa4c_download=debug,aa4c_core=debug")
        .try_init();

    fn require_on_path(name_without_ext: &str) -> PathBuf {
        let path_var = std::env::var_os("PATH").unwrap_or_default();
        let exe_name = format!("{name_without_ext}{}", std::env::consts::EXE_SUFFIX);
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(&exe_name);
            if candidate.is_file() {
                return candidate;
            }
        }
        panic!("{exe_name} not found in PATH — install it to run this test (see HANDOFF.md)");
    }

    let dir = tempfile::tempdir().unwrap();
    let download_dir = dir.path().join("test-downloads");
    {
        let seed_store = aa4c_store::Store::open(&dir.path().join("aa4c.db"))
            .await
            .unwrap();
        seed_store
            .set_setting(
                "download_dir",
                &serde_json::to_string(&download_dir.to_string_lossy().into_owned()).unwrap(),
            )
            .await
            .unwrap();
    }

    let mut config = CoreConfig::new(dir.path().to_path_buf());
    config.listen_port = 0;
    config.transfer.default_save_dir = dir.path().join("downloads");
    // mDNS 对本套件只是噪声：全部用例都用显式地址建连（见文件头注释），而每个
    // `ServiceDaemon` 自带一条 OS 线程 + 一组 5353 组播 socket，并行跑十几个用例时
    // 几十个守护进程互相挤，配对会开始超时。
    config.disable_discovery = true;
    // 端口映射的真实实现会去改**跑测试这台机器所在路由器**的配置。有几个用例会打开
    // `enable_remote`（映射的外层闸），只是碰巧在退避周期内就跑完了才没真发出去——
    // 不能靠这种巧合，直接从结构上关掉。
    config.disable_port_mapping = true;
    config.plugins.register(Arc::new(DownloadPlugin::new(
        Arc::new(ProcessSpawner::new(require_on_path("aria2c"))),
        Some(Arc::new(ProcessSpawner::new(require_on_path(
            "transmission-daemon",
        )))),
    )));
    let core = Core::start(config).await.expect("core starts");

    let magnet =
        "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=aa4c-core-e2e-test";
    let id = from_json::<String>(
        core.plugin_invoke("download", "add".into(), json!({ "url": magnet }))
            .await
            .unwrap(),
    );
    assert_eq!(id.len(), 40, "BT task id should be the 40-hex infohash");

    let listed = from_json::<Vec<aa4c_types::DownloadTask>>(
        core.plugin_invoke("download", "list".into(), json!({}))
            .await
            .unwrap(),
    );
    let task = listed
        .iter()
        .find(|t| t.id == id)
        .expect("task should be listed after add_download");
    assert_eq!(task.kind, aa4c_types::DownloadKind::Bt);

    core.plugin_invoke("download", "pause".into(), json!({ "id": id }))
        .await
        .unwrap();
    core.plugin_invoke("download", "resume".into(), json!({ "id": id }))
        .await
        .unwrap();
    core.plugin_invoke(
        "download",
        "cancel".into(),
        json!({ "id": id, "delete_local": false }),
    )
    .await
    .unwrap();

    let listed = from_json::<Vec<aa4c_types::DownloadTask>>(
        core.plugin_invoke("download", "list".into(), json!({}))
            .await
            .unwrap(),
    );
    let task = listed
        .iter()
        .find(|t| t.id == id)
        .expect("task should still be listed after cancel");
    assert_eq!(task.status, aa4c_types::DownloadStatus::Removed);

    core.shutdown().await.unwrap();
}

//! aa4c-download D2.3 集成测试：真实 `transmission-daemon`（PATH 里的系统
//! 安装）+ `TransmissionClient`，验证 HTTP + `X-Transmission-Session-Id`
//! 握手、鉴权、RPC 调用往返都能对上真实进程——不是拿手写 mock server 假装。
//!
//! 需要本机 PATH 里有 `transmission-daemon`（macOS: `brew install
//! transmission-cli`；Linux: `apt install transmission-daemon`；Windows:
//! 见 `.github/workflows/ci.yml` 的官方 MSI 解包提取步骤）——找不到时显式
//! panic 报安装指引，不静默跳过（同 D1/D2.4 集成测试的既有惯例）。

use std::path::PathBuf;

use aa4c_download::{ProcessSpawner, TransmissionClient, TransmissionProcess};
use serde_json::json;

fn require_transmission_daemon() -> PathBuf {
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    let exe_name = format!("transmission-daemon{}", std::env::consts::EXE_SUFFIX);
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(&exe_name);
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!(
        "transmission-daemon not found in PATH — install it to run this test \
         (macOS: `brew install transmission-cli`; Linux: `apt install transmission-daemon`). \
         See HANDOFF.md environment setup / .github/workflows/ci.yml for Windows."
    );
}

async fn spawn_daemon() -> (TransmissionProcess, tempfile::TempDir) {
    let bin = require_transmission_daemon();
    let spawner = ProcessSpawner::new(bin);
    let dir = tempfile::tempdir().unwrap();
    let download_dir = dir.path().join("downloads");
    let proc =
        TransmissionProcess::spawn(&spawner, dir.path(), &download_dir, None, None, None, None)
            .await
            .expect("transmission-daemon should spawn");
    // 前台模式启动很快，但 RPC 端口真正开始监听还有一个短窗口——同 D1 aria2
    // 健康检查的思路，这里给个固定的启动缓冲（D2.3 范围内先不做重试轮询，
    // D2.5 接入 Core 编排时应该照 aria2 `connect_and_health_check` 的形状补上）。
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    (proc, dir)
}

#[tokio::test]
async fn session_handshake_then_call_succeeds() {
    let (proc, _dir) = spawn_daemon().await;
    let client = TransmissionClient::new(proc.port, &proc.username, &proc.password);

    // 第一次调用必然先撞上 409（没有 session id），客户端应该自动取到 id 并
    // 重试成功——从调用方视角看不出这个中间过程，直接拿到正常响应。
    let result = client
        .call("session-get", json!({}))
        .await
        .expect("session-get should succeed after the 409 handshake");
    assert!(
        result.get("rpc-version").is_some(),
        "session-get response should include rpc-version, got: {result}"
    );

    // 第二次调用复用缓存的 session id（不应该再触发一次 409 才能拿到结果——
    // 从外部只能验证"还是能成功"，缓存本身是内部实现细节）。
    let result2 = client
        .call("session-get", json!({}))
        .await
        .expect("second call should also succeed");
    assert_eq!(result["rpc-version"], result2["rpc-version"]);

    proc.kill().await;
}

#[tokio::test]
async fn wrong_credentials_are_rejected() {
    let (proc, _dir) = spawn_daemon().await;
    let client = TransmissionClient::new(proc.port, &proc.username, "definitely-wrong-password");

    let err = client
        .call("session-get", json!({}))
        .await
        .expect_err("wrong password must not succeed");
    assert!(
        err.to_string().contains("401"),
        "expected a 401 error, got: {err}"
    );

    proc.kill().await;
}

#[tokio::test]
async fn torrent_add_and_remove_round_trip() {
    let (proc, _dir) = spawn_daemon().await;
    let client = TransmissionClient::new(proc.port, &proc.username, &proc.password);

    // 语法合法的 magnet（40 位十六进制 infohash）就足够让 transmission-daemon
    // 接受并建档——RPC 调用本身不需要真的连上任何 peer/tracker。
    let magnet = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=aa4c-test";
    let add_result = client
        .call("torrent-add", json!({ "filename": magnet, "paused": true }))
        .await
        .expect("torrent-add should succeed");

    let hash = add_result["torrent-added"]["hashString"]
        .as_str()
        .or_else(|| add_result["torrent-duplicate"]["hashString"].as_str())
        .expect("response should contain a hashString under torrent-added or torrent-duplicate")
        .to_string();
    assert_eq!(hash.len(), 40);

    let remove_result = client
        .call(
            "torrent-remove",
            json!({ "ids": [hash], "delete-local-data": false }),
        )
        .await
        .expect("torrent-remove should succeed");
    assert!(remove_result.is_object());

    proc.kill().await;
}

//! aa4c-download 集成测试：真实 aria2c（PATH 里的系统安装）+ 手写回环 HTTP
//! 静态文件服务器（同 aa4c-core 里 C1 断点续传测试用 UDP 黑洞代理的先例：
//! 测试基础设施手写，不为测试引入额外依赖）。
//!
//! 需要本机 PATH 里有 `aria2c`（`brew install aria2` / `apt install aria2` /
//! `choco install aria2`）——找不到时显式 panic 报安装指引，不静默跳过
//! （V0.4_IMPLEMENTATION_PLAN.md D1 步骤 4 的明确要求）。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use aa4c_download::{DownloadService, ProcessSpawner};
use aa4c_store::Store;
use aa4c_types::{CoreEvent, DownloadStatus, DownloadTask};
use tokio::sync::broadcast;

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
    panic!(
        "aria2c not found in PATH — install it to run aa4c-download tests \
         (macOS: `brew install aria2`; Linux: `apt install aria2`; \
         Windows: `choco install aria2`). See HANDOFF.md environment setup."
    );
}

/// 立即整体返回响应体的回环 HTTP 服务器；`/missing` 返回 404。
async fn spawn_fast_http_server(body: Vec<u8>) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(serve_one(stream, body.clone(), None));
        }
    });
    addr
}

/// 按小块 + 固定间隔吐出响应体的回环 HTTP 服务器——制造一个真实存在的传输
/// 时间窗口，供暂停/取消/中途重启测试在其间发起操作。
async fn spawn_slow_http_server(body: Vec<u8>, chunk_delay: Duration) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(serve_one(stream, body.clone(), Some(chunk_delay)));
        }
    });
    addr
}

/// 处理一条连接。支持 `Range: bytes=N-` 请求（206 Partial Content）——暂停/
/// 继续与跨重启续传都要求服务器能从任意偏移续吐，没有这个真实的 aria2 只会
/// 检测到服务器不支持续传后整体重下（功能上也能跑完，但测的就不是"续传"了）。
async fn serve_one(mut stream: tokio::net::TcpStream, body: Vec<u8>, throttle: Option<Duration>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        let n = match stream.read(&mut chunk).await {
            Ok(n) if n > 0 => n,
            _ => return,
        };
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.len() > 8192 {
            break;
        }
    }
    let request = String::from_utf8_lossy(&buf);
    let mut lines = request.lines();
    let path = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_string();
    if path == "/missing" {
        let _ = stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await;
        return;
    }

    let range_start: Option<usize> = lines
        .find(|l| l.to_ascii_lowercase().starts_with("range:"))
        .and_then(|l| l.split("bytes=").nth(1))
        .and_then(|r| r.split('-').next())
        .and_then(|s| s.trim().parse().ok());
    let start = range_start.unwrap_or(0).min(body.len());
    let slice = &body[start..];

    let header = match range_start {
        Some(_) => format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {}-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            start,
            body.len().saturating_sub(1),
            body.len(),
            slice.len()
        ),
        None => format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
            body.len()
        ),
    };
    if stream.write_all(header.as_bytes()).await.is_err() {
        return;
    }
    match throttle {
        None => {
            let _ = stream.write_all(slice).await;
        }
        Some(delay) => {
            for c in slice.chunks(32 * 1024) {
                if stream.write_all(c).await.is_err() {
                    return;
                }
                tokio::time::sleep(delay).await;
            }
        }
    }
}

async fn start_service(
    data_dir: &std::path::Path,
    download_dir: &std::path::Path,
) -> (
    Arc<DownloadService>,
    broadcast::Receiver<CoreEvent>,
    Store,
    Arc<ProcessSpawner>,
) {
    let store = Store::open(&data_dir.join("aa4c.db")).await.unwrap();
    let (events, rx) = broadcast::channel(64);
    let spawner = Arc::new(ProcessSpawner::new(require_aria2c()));
    let svc = DownloadService::start(
        spawner.clone(),
        store.clone(),
        events,
        data_dir.to_path_buf(),
        download_dir.to_path_buf(),
    )
    .await;
    (svc, rx, store, spawner)
}

async fn wait_for_status(
    store: &Store,
    id: &str,
    want: DownloadStatus,
    timeout: Duration,
) -> DownloadTask {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Some(task) = store.get_download(id).await.unwrap() {
            if task.status == want {
                return task;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timed out waiting for status {want:?} on task {id}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn full_download_completes_with_correct_content() {
    let dir = tempfile::tempdir().unwrap();
    let download_dir = dir.path().join("downloads");
    std::fs::create_dir_all(&download_dir).unwrap();
    let body = b"AA4C download center D1 integration test payload. ".repeat(500);
    let addr = spawn_fast_http_server(body.clone()).await;
    let (svc, _rx, store, _spawner) = start_service(dir.path(), &download_dir).await;

    let id = svc.add(format!("http://{addr}/file.bin")).await.unwrap();
    let task = wait_for_status(
        &store,
        &id,
        DownloadStatus::Complete,
        Duration::from_secs(20),
    )
    .await;

    let save_path = task.save_path.expect("save_path set on completion");
    let content = std::fs::read(&save_path).unwrap();
    assert_eq!(content, body);
    assert_eq!(task.total_bytes, body.len() as u64);
    assert_eq!(task.downloaded_bytes, body.len() as u64);

    svc.shutdown().await;
}

#[tokio::test]
async fn pause_then_resume_reaches_complete() {
    let dir = tempfile::tempdir().unwrap();
    let download_dir = dir.path().join("downloads");
    std::fs::create_dir_all(&download_dir).unwrap();
    let body = vec![7u8; 2 * 1024 * 1024];
    let addr = spawn_slow_http_server(body.clone(), Duration::from_millis(80)).await;
    let (svc, _rx, store, _spawner) = start_service(dir.path(), &download_dir).await;

    let id = svc.add(format!("http://{addr}/file.bin")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    svc.pause(id.clone()).await.unwrap();
    wait_for_status(&store, &id, DownloadStatus::Paused, Duration::from_secs(10)).await;

    svc.resume(id.clone()).await.unwrap();
    let task = wait_for_status(
        &store,
        &id,
        DownloadStatus::Complete,
        Duration::from_secs(20),
    )
    .await;
    let content = std::fs::read(task.save_path.unwrap()).unwrap();
    assert_eq!(content, body);

    svc.shutdown().await;
}

#[tokio::test]
async fn cancel_marks_removed_and_engine_does_not_resurrect_it() {
    let dir = tempfile::tempdir().unwrap();
    let download_dir = dir.path().join("downloads");
    std::fs::create_dir_all(&download_dir).unwrap();
    let body = vec![9u8; 2 * 1024 * 1024];
    let addr = spawn_slow_http_server(body, Duration::from_millis(80)).await;
    let (svc, _rx, store, _spawner) = start_service(dir.path(), &download_dir).await;

    let id = svc.add(format!("http://{addr}/file.bin")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    svc.cancel(id.clone()).await.unwrap();

    let task = store.get_download(&id).await.unwrap().unwrap();
    assert_eq!(task.status, DownloadStatus::Removed);

    // 等过至少一个对账节拍，确认后台轮询不会把它救回成别的状态
    tokio::time::sleep(Duration::from_secs(3)).await;
    let after = store.get_download(&id).await.unwrap().unwrap();
    assert_eq!(after.status, DownloadStatus::Removed);

    svc.shutdown().await;
}

#[tokio::test]
async fn missing_url_transitions_to_error_with_human_readable_message() {
    let dir = tempfile::tempdir().unwrap();
    let download_dir = dir.path().join("downloads");
    std::fs::create_dir_all(&download_dir).unwrap();
    let addr = spawn_fast_http_server(b"unused".to_vec()).await;
    let (svc, _rx, store, _spawner) = start_service(dir.path(), &download_dir).await;

    let id = svc.add(format!("http://{addr}/missing")).await.unwrap();
    let task = wait_for_status(&store, &id, DownloadStatus::Error, Duration::from_secs(15)).await;
    assert!(task.error.is_some());

    svc.shutdown().await;
}

/// DoD 第 2 条的直接验证：应用退出重启后，未完成的下载自动恢复——续传数据归
/// aria2（`save-session`/`input-file`），任务记录归 AA4C（同一个 GID）。
#[tokio::test]
async fn task_resumes_across_service_restart_with_same_gid() {
    let dir = tempfile::tempdir().unwrap();
    let download_dir = dir.path().join("downloads");
    std::fs::create_dir_all(&download_dir).unwrap();
    let body = vec![3u8; 2 * 1024 * 1024];
    let addr = spawn_slow_http_server(body.clone(), Duration::from_millis(80)).await;

    let store = Store::open(&dir.path().join("aa4c.db")).await.unwrap();
    let spawner = Arc::new(ProcessSpawner::new(require_aria2c()));

    let (events, _rx) = broadcast::channel(64);
    let svc = DownloadService::start(
        spawner.clone(),
        store.clone(),
        events,
        dir.path().to_path_buf(),
        download_dir.clone(),
    )
    .await;
    let id = svc.add(format!("http://{addr}/file.bin")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    svc.shutdown().await; // 触发一次 session 保存 + 干净关闭子进程

    // 模拟应用重启：同一个 data_dir/Store，新的 DownloadService。
    let (events2, _rx2) = broadcast::channel(64);
    let svc2 = DownloadService::start(
        spawner,
        store.clone(),
        events2,
        dir.path().to_path_buf(),
        download_dir,
    )
    .await;

    let task = wait_for_status(
        &store,
        &id,
        DownloadStatus::Complete,
        Duration::from_secs(20),
    )
    .await;
    assert_eq!(task.id, id, "GID must survive the restart unchanged");
    let content = std::fs::read(task.save_path.unwrap()).unwrap();
    assert_eq!(content, body);

    svc2.shutdown().await;
}

/// §3.4 对账逻辑：session 文件丢失（损坏/被手动删）→ 表里遗留的未完态记录
/// 标记为失败，而不是永远卡在"传输中"。
#[tokio::test]
async fn missing_session_file_marks_orphaned_task_as_error() {
    let dir = tempfile::tempdir().unwrap();
    let download_dir = dir.path().join("downloads");
    std::fs::create_dir_all(&download_dir).unwrap();
    let body = vec![5u8; 2 * 1024 * 1024];
    let addr = spawn_slow_http_server(body, Duration::from_millis(80)).await;

    let store = Store::open(&dir.path().join("aa4c.db")).await.unwrap();
    let spawner = Arc::new(ProcessSpawner::new(require_aria2c()));

    let (events, _rx) = broadcast::channel(64);
    let svc = DownloadService::start(
        spawner.clone(),
        store.clone(),
        events,
        dir.path().to_path_buf(),
        download_dir.clone(),
    )
    .await;
    let id = svc.add(format!("http://{addr}/file.bin")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    svc.shutdown().await;

    std::fs::remove_file(dir.path().join("aria2.session")).unwrap();

    let (events2, _rx2) = broadcast::channel(64);
    let svc2 = DownloadService::start(
        spawner,
        store.clone(),
        events2,
        dir.path().to_path_buf(),
        download_dir,
    )
    .await;

    let task = wait_for_status(&store, &id, DownloadStatus::Error, Duration::from_secs(5)).await;
    assert!(task.error.is_some());

    svc2.shutdown().await;
}

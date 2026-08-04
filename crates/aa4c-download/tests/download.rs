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

use aa4c_download::{DownloadLimits, DownloadService, ProcessSpawner};
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
        None,
        store.clone(),
        events,
        data_dir.to_path_buf(),
        download_dir.to_path_buf(),
        DownloadLimits::default(),
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
        None,
        store.clone(),
        events,
        dir.path().to_path_buf(),
        download_dir.clone(),
        DownloadLimits::default(),
    )
    .await;
    let id = svc.add(format!("http://{addr}/file.bin")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    svc.shutdown().await; // 触发一次 session 保存 + 干净关闭子进程

    // 模拟应用重启：同一个 data_dir/Store，新的 DownloadService。
    let (events2, _rx2) = broadcast::channel(64);
    let svc2 = DownloadService::start(
        spawner,
        None,
        store.clone(),
        events2,
        dir.path().to_path_buf(),
        download_dir,
        DownloadLimits::default(),
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

/// "同时下载数"（`DownloadLimits::concurrency`）真的会限制 aria2 实际并发跑
/// 的任务数，不是只写进 conf 文件摆设：起 3 个慢速下载、把并发数设成 1，全程
/// 轮询 `list_downloads()`，断言任意时刻 `Active` 的任务数都不超过 1（其余
/// 排在 `Waiting`），最终三个都能跑完——`max-concurrent-downloads` 是 aria2
/// 自己的全局限流，AA4C 这边只是把设置页的数字原样透传（见 `conf.rs`），这条
/// 测试验证的是"透传之后 aria2 真的照办"，不是 AA4C 自己实现了限流。
#[tokio::test]
async fn download_concurrency_limit_caps_simultaneous_active_tasks() {
    let dir = tempfile::tempdir().unwrap();
    let download_dir = dir.path().join("downloads");
    std::fs::create_dir_all(&download_dir).unwrap();
    let store = Store::open(&dir.path().join("aa4c.db")).await.unwrap();
    let spawner = Arc::new(ProcessSpawner::new(require_aria2c()));
    let (events, _rx) = broadcast::channel(64);
    let svc = DownloadService::start(
        spawner,
        None,
        store.clone(),
        events,
        dir.path().to_path_buf(),
        download_dir.clone(),
        DownloadLimits {
            concurrency: Some(1),
            ..DownloadLimits::default()
        },
    )
    .await;

    let body = vec![13u8; 512 * 1024];
    let mut ids = Vec::new();
    for i in 0..3 {
        let addr = spawn_slow_http_server(body.clone(), Duration::from_millis(60)).await;
        let id = svc.add(format!("http://{addr}/file{i}.bin")).await.unwrap();
        ids.push(id);
    }

    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut max_active_seen = 0usize;
    loop {
        let tasks = store.list_downloads().await.unwrap();
        let active = tasks
            .iter()
            .filter(|t| ids.contains(&t.id) && t.status == DownloadStatus::Active)
            .count();
        max_active_seen = max_active_seen.max(active);
        assert!(
            active <= 1,
            "concurrency=1 but saw {active} tasks Active at once"
        );
        if ids.iter().all(|id| {
            tasks
                .iter()
                .any(|t| &t.id == id && t.status == DownloadStatus::Complete)
        }) {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("not all downloads completed within the deadline: {tasks:?}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // 三个任务分三批跑（并发=1），至少要真的观察到"同时有一个在跑"过，否则
    // 上面 `active <= 1` 的断言就是空转——确认这条测试真的锻炼到了限流路径。
    assert_eq!(max_active_seen, 1);

    svc.shutdown().await;
}

/// 通过命令行匹配（含本次测试独占的 `data_dir` 路径，天然唯一）找到刚才拉起的
/// aria2c 子进程并 SIGKILL——模拟"进程本身崩溃退出"而不是"连接断开"，两者对
/// `actor_loop` 触发的路径不同（见 `crates/aa4c-download/src/lib.rs`
/// `connected.client.closed()` 分支）。只在 unix 上实现：Windows 上按命令行
/// 匹配进程需要额外依赖（`sysinfo` 之类），这个 crate 的测试基础设施一贯手写、
/// 不为测试引入依赖（见文件头注释），跨平台匹配值得单开一次评估，不在这次
/// 顺带解决。
#[cfg(unix)]
fn kill_process_matching(pattern: &str) {
    let out = std::process::Command::new("pgrep")
        .arg("-f")
        .arg(pattern)
        .output()
        .expect("pgrep must be available on unix test runners");
    let pids: Vec<&str> = std::str::from_utf8(&out.stdout)
        .unwrap()
        .lines()
        .filter(|l| !l.is_empty())
        .collect();
    assert!(
        !pids.is_empty(),
        "no process matched {pattern:?} — aria2c may not have started yet"
    );
    for pid in pids {
        let _ = std::process::Command::new("kill")
            .args(["-9", pid])
            .status();
    }
}

/// 对应 DOWNLOAD_DESIGN.md「仍待实现/后续」里点名的缺口："子进程崩溃后的
/// 自动重启策略"。杀掉 aria2c 真实进程（不是断连接），断言：①原任务不会永远
/// 卡在 Active/Waiting（无论是续上完成还是因 session 未及时保存被对账标失败，
/// 都要有一个终态，不能悄悄挂住）；②服务本身自愈——崩溃之后新建的下载任务
/// 依然能正常跑完，证明 `actor_loop` 真的重新拉起了一个可用的 aria2c，而不是
/// 从此在本次会话里永久不可用。
#[cfg(unix)]
#[tokio::test]
async fn aria2_crash_mid_download_recovers_and_new_downloads_still_work() {
    let dir = tempfile::tempdir().unwrap();
    let download_dir = dir.path().join("downloads");
    std::fs::create_dir_all(&download_dir).unwrap();
    let body = vec![11u8; 4 * 1024 * 1024];
    let addr = spawn_slow_http_server(body, Duration::from_millis(80)).await;
    let (svc, _rx, store, _spawner) = start_service(dir.path(), &download_dir).await;

    let id = svc.add(format!("http://{addr}/file.bin")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    let data_dir_str = dir.path().to_string_lossy().into_owned();
    kill_process_matching(&format!("aria2c.*{data_dir_str}"));

    // 原任务最终必须有一个终态，不能永远悬在 Active/Waiting——不关心具体是
    // 哪个终态（取决于 aria2 session 上次自动保存的时间点，见
    // `save-session-interval` 的 30 秒周期，跟这次崩溃时机无关，不是这个
    // 测试要断言的东西）。重连退避（5 次，最长到 8 秒一次）+ respawn 健康检查
    // 一整套跑下来留足余量。
    let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
    loop {
        let task = store.get_download(&id).await.unwrap().unwrap();
        if !matches!(
            task.status,
            DownloadStatus::Active | DownloadStatus::Waiting
        ) {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("original task never left Active/Waiting after aria2c crash");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // 真正的重点：崩溃之后，服务还能不能正常收新任务——这才是区分"自愈"和
    // "本次会话下载能力永久报废"的地方。文件名故意跟上面那个不同
    // （`file2.bin` vs `file.bin`）——两个任务共用同一个 `download_dir`，撞同名
    // 会让 aria2 把这次全新下载误当成对前一个中断任务遗留的 `.aria2` 控制文件
    // 续传，服务器返回的内容对不上导致 aria2 直接 abort（排查过：这是测试本身
    // 的假阳性，不是 respawn 逻辑的问题）。
    let body2 = b"post-crash recovery payload".repeat(200);
    let addr2 = spawn_fast_http_server(body2.clone()).await;
    let id2 = svc
        .add(format!("http://{addr2}/file2.bin"))
        .await
        .expect("service must still accept new downloads after respawn");
    let task2 = wait_for_status(
        &store,
        &id2,
        DownloadStatus::Complete,
        Duration::from_secs(20),
    )
    .await;
    let content2 = std::fs::read(task2.save_path.unwrap()).unwrap();
    assert_eq!(content2, body2);

    svc.shutdown().await;
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
        None,
        store.clone(),
        events,
        dir.path().to_path_buf(),
        download_dir.clone(),
        DownloadLimits::default(),
    )
    .await;
    let id = svc.add(format!("http://{addr}/file.bin")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    svc.shutdown().await;

    std::fs::remove_file(dir.path().join("aria2.session")).unwrap();

    let (events2, _rx2) = broadcast::channel(64);
    let svc2 = DownloadService::start(
        spawner,
        None,
        store.clone(),
        events2,
        dir.path().to_path_buf(),
        download_dir,
        DownloadLimits::default(),
    )
    .await;

    let task = wait_for_status(&store, &id, DownloadStatus::Error, Duration::from_secs(5)).await;
    assert!(task.error.is_some());

    svc2.shutdown().await;
}

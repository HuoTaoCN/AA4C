//! 孤儿进程防护（DOWNLOAD_DESIGN.md §3.6.2）：Transmission 没有 aria2 那样的
//! `stop-with-process` 内建选项，AA4C 自己保证异常退出（崩溃/被强杀，来不及
//! 跑任何清理代码）时不留下孤儿子进程。三个机制都已用真实环境 PoC 验证过（父
//! 进程 `std::process::abort()` 模拟不可控崩溃，检查子进程是否被自动清理，
//! 详见 DOWNLOAD_DESIGN.md §3.6.2/§9）：
//!
//! - **Linux**：[`arm_pdeathsig`] 在子进程 `exec` 前调用
//!   `prctl(PR_SET_PDEATHSIG, SIGKILL)`——这个只能在子进程自己 `exec` 前调用
//!   （`pre_exec` 钩子），所以只有 `ProcessSpawner`（`tokio::process::Command`）
//!   能用；Tauri sidecar 机制（`tauri_plugin_shell::process::CommandChild`）
//!   不给 spawn 时注入钩子的余地（已查证：这个类型只公开 `write`/`kill`/`pid`
//!   三个方法），Tauri 生产路径的 Linux 桌面退化用下面的 PID 文件方案（同
//!   macOS）。真实 `ubuntu-latest` CI runner 上验证通过。
//! - **Windows**：[`protect_with_job_object`] 用 `CreateJobObjectW` +
//!   `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` + `AssignProcessToJobObject`，
//!   按 PID 事后归属——不需要在 spawn 时注入任何钩子，`ProcessSpawner`/
//!   `TauriSidecarSpawner` 两条路径统一适用（只要能拿到子进程 PID，见
//!   `EngineChild::pid`）。宿主进程异常终止时 OS 回收其持有的全部句柄
//!   （含 Job 句柄），触发"最后一个句柄关闭"，内核自动杀光 Job 里的全部
//!   进程，不需要宿主自己活着执行任何代码。真实 `windows-latest` CI
//!   runner 上验证通过。
//! - **macOS（以及 Tauri 路径下的 Linux）**：没有内核层等价机制，
//!   [`OrphanPidfile`] 提供 PID 文件 + 下次启动清扫兜底——写文件时记录
//!   `pid` + 进程身份（启动时间 + 进程名）；下次启动读文件后先核对身份完全
//!   匹配才动手 kill，核对失败则拒绝清理、静默跳过（防止 PID 复用误杀无关
//!   进程）。本机验证过正常清扫与防误杀两条路径。
//!
//! 全部函数都是尽力而为、失败只记日志——同其余可选能力的降级惯例，不阻塞
//! 调用方主流程。

use std::path::PathBuf;

/// Linux 上给即将 spawn 的命令装 `PR_SET_PDEATHSIG`；其余平台是 no-op。
/// 无条件对任意引擎调用是安全的（对已有 `stop-with-process` 的 aria2 只是
/// 多一层免费的保险，对没有等价内建选项的 Transmission 是必需的）。
pub(crate) fn arm_pdeathsig(cmd: &mut tokio::process::Command) {
    #[cfg(target_os = "linux")]
    linux::arm(cmd);
    #[cfg(not(target_os = "linux"))]
    let _ = cmd;
}

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
mod linux {
    // 不为一个系统调用引入 libc 依赖：手写的 FFI 声明已经在真实 CI 上验证过
    // 正确（见 poc/orphan-protection 分支历史，已删除），签名照抄。
    extern "C" {
        fn prctl(option: i32, arg2: u64, arg3: u64, arg4: u64, arg5: u64) -> i32;
    }
    const PR_SET_PDEATHSIG: i32 = 1;
    const SIGKILL: u64 = 9;

    pub(super) fn arm(cmd: &mut tokio::process::Command) {
        // SAFETY: 这个闭包在 fork 之后、exec 之前的子进程里运行（`pre_exec`
        // 的固有约定）。它只调用一个不分配内存、不触碰共享状态、失败只是
        // 返回 errno 的系统调用（`prctl`），不违反 `pre_exec` 文档警告的
        // async-signal-safety 陷阱（那些陷阱针对的是分配内存/加锁这类操作）。
        unsafe {
            cmd.pre_exec(|| {
                if prctl(PR_SET_PDEATHSIG, SIGKILL, 0, 0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
}

/// Windows 上把 `pid` 对应的进程绑进一个"宿主消失即被杀"的 Job Object。
/// 幂等/尽力而为——失败只记日志，不返回错误、不阻塞调用方。
pub(crate) fn protect_with_job_object(pid: u32) {
    #[cfg(windows)]
    windows::protect(pid);
    #[cfg(not(windows))]
    let _ = pid;
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    };

    pub(super) fn protect(pid: u32) {
        // SAFETY: 全部是标准 Win32 调用，参数都是本函数局部构造的有效值
        // （`info` 用 `zeroed()` 初始化后只设了一个已知字段，符合
        // `JOBOBJECT_EXTENDED_LIMIT_INFORMATION` 的 FFI 契约）；每个句柄
        // 用完（或失败提前返回前）都会 `CloseHandle`。已在真实 windows-latest
        // CI runner 上端到端验证：父进程 abort 后子进程被自动杀死。
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                tracing::warn!(
                    error = ?std::io::Error::last_os_error(),
                    "orphan_guard: CreateJobObjectW failed, Windows orphan protection disabled for this process"
                );
                return;
            }

            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let set_ok = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if set_ok == 0 {
                tracing::warn!(
                    error = ?std::io::Error::last_os_error(),
                    "orphan_guard: SetInformationJobObject failed, Windows orphan protection disabled for this process"
                );
                CloseHandle(job);
                return;
            }

            let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
            if process.is_null() {
                tracing::warn!(
                    pid,
                    error = ?std::io::Error::last_os_error(),
                    "orphan_guard: OpenProcess failed, Windows orphan protection disabled for this process"
                );
                CloseHandle(job);
                return;
            }

            let assigned = AssignProcessToJobObject(job, process);
            CloseHandle(process);
            if assigned == 0 {
                tracing::warn!(
                    pid,
                    error = ?std::io::Error::last_os_error(),
                    "orphan_guard: AssignProcessToJobObject failed, Windows orphan protection disabled for this process"
                );
                CloseHandle(job);
                return;
            }

            // 故意不 CloseHandle(job)：Job 对象必须活到本进程（AA4C 自己）
            // 退出为止——这正是 KILL_ON_JOB_CLOSE 依赖的机制（本进程持有的
            // 全部句柄在进程终止时被 OS 统一回收，那一刻触发"最后一个句柄
            // 关闭"）。提前关掉这个句柄会让保护立即失效。`job` 是裸指针
            // （`*mut c_void`，`Copy`），本来就没有 `Drop`——"不清理"就是
            // 什么都不做，不需要（也不能）用 `mem::forget` 表达这个意图，
            // 那对 `Copy` 类型是空操作（clippy 会警告，之前误加了）。
        }
    }
}

/// macOS（以及 Tauri 路径下的 Linux）孤儿进程兜底：PID 文件记录子进程身份，
/// 下次启动核对身份匹配后才清理，防 PID 复用误杀无关进程。
pub(crate) struct OrphanPidfile {
    path: PathBuf,
}

impl OrphanPidfile {
    pub(crate) fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// 记录当前子进程的身份。失败只记日志——写不进 PID 文件不影响下载功能
    /// 本身，只是弱化了"下次启动清扫"这一层兜底。
    pub(crate) fn record(&self, pid: u32) {
        let Some(identity) = process_identity(pid) else {
            tracing::warn!(
                pid,
                "orphan_guard: could not read process identity right after spawn, skipping pidfile write"
            );
            return;
        };
        let body = format!("pid={pid}\nidentity={identity}\n");
        if let Err(e) = std::fs::write(&self.path, body) {
            tracing::warn!(error = %e, path = ?self.path, "orphan_guard: failed to write pidfile");
        }
    }

    /// 清除本次正常关闭已经处理过的记录，避免下次启动误把一个已经善终的
    /// 进程当成孤儿去核对（虽然核对本身是安全的，纯粹是省一次无意义 I/O）。
    pub(crate) fn clear(&self) {
        let _ = std::fs::remove_file(&self.path);
    }

    /// 启动时调用：读上次留下的 PID 文件，身份匹配才清理，然后无论如何都
    /// 删除这个文件（不匹配也不用留着——留着不会被再次使用，只是一个陈旧
    /// 记录，删掉避免误导下次读取的人）。
    pub(crate) fn sweep(&self) {
        let Ok(body) = std::fs::read_to_string(&self.path) else {
            return; // 没有上次遗留的文件，没什么可清扫的（正常路径）
        };
        let _ = std::fs::remove_file(&self.path);

        let mut pid: Option<u32> = None;
        let mut identity: Option<String> = None;
        for line in body.lines() {
            if let Some(v) = line.strip_prefix("pid=") {
                pid = v.parse().ok();
            } else if let Some(v) = line.strip_prefix("identity=") {
                identity = Some(v.to_string());
            }
        }
        let (Some(pid), Some(expected)) = (pid, identity) else {
            return;
        };

        match process_identity(pid) {
            Some(actual) if actual == expected => {
                tracing::info!(
                    pid,
                    "orphan_guard: sweeping orphaned process from a previous crash"
                );
                kill_pid(pid);
            }
            Some(actual) => {
                tracing::debug!(
                    pid,
                    expected,
                    actual,
                    "orphan_guard: pidfile identity mismatch (pid reused by an unrelated process), refusing to kill"
                );
            }
            None => {
                // 进程已经不在了（正常退出、或者已经被清理过），无事可做。
            }
        }
    }
}

#[cfg(unix)]
fn kill_pid(pid: u32) {
    // 不引入额外依赖：直接 shell 出去，同 macOS 身份核对复用的思路（这条
    // 路径本来就只在"上次崩溃遗留孤儿"这种低频场景触发，不是热路径）。
    let _ = std::process::Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .status();
}

#[cfg(windows)]
fn kill_pid(pid: u32) {
    let _ = std::process::Command::new("taskkill")
        .args(["/F", "/PID", &pid.to_string()])
        .status();
}

/// 进程身份指纹：`pid` 本身会被复用，只有"pid + 启动时间 + 进程名"三者一起
/// 才能安全地判断"这就是我当初记录的那个进程"。返回 `None` 表示进程已经
/// 不存在了。
#[cfg(target_os = "macos")]
fn process_identity(pid: u32) -> Option<String> {
    let out = std::process::Command::new("ps")
        .args(["-o", "lstart=,comm=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Linux 的 Tauri 路径兜底：直接读 `/proc`，不 shell 出去（比 macOS 分支更
/// 直接——Linux 没有"进程信息只能靠 ps 解析"这个限制）。`/proc/<pid>/stat`
/// 的 `comm` 字段（第 2 个、带括号）与 `starttime` 字段（第 22 个，自系统
/// 启动以来的 tick 数）拼在一起当身份指纹。
#[cfg(target_os = "linux")]
fn process_identity(pid: u32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // comm 字段本身可能含空格，用最后一个 ')' 定位，不能简单按空格切分。
    let comm_end = stat.rfind(')')?;
    let comm_start = stat.find('(')?;
    let comm = stat.get(comm_start..=comm_end)?;
    let rest: Vec<&str> = stat.get(comm_end + 1..)?.split_whitespace().collect();
    // 第 3 个字段起是 `rest[0]`；starttime 是整体第 22 个字段 = rest[18]。
    let starttime = rest.get(18).copied().unwrap_or("");
    Some(format!("{comm} {starttime}"))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn process_identity(pid: u32) -> Option<String> {
    // Windows 走 Job Object，不依赖这条兜底路径；给个保守实现（只按 PID 是否
    // 存在判断，不做身份核对）而不是 `unimplemented!`，避免这个 crate 在
    // 未来任何新增平台上编译不过。
    let exists = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}")])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
        .unwrap_or(false);
    exists.then(|| pid.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn spawn_standin() -> (std::process::Child, u32) {
        let child = if cfg!(windows) {
            Command::new("cmd")
                .args(["/C", "ping -n 60 127.0.0.1 >NUL"])
                .spawn()
                .unwrap()
        } else {
            Command::new("sleep").arg("60").spawn().unwrap()
        };
        let pid = child.id();
        (child, pid)
    }

    #[test]
    fn sweep_kills_matching_orphan_and_removes_pidfile() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("engine.pid");
        let (mut child, pid) = spawn_standin();
        std::thread::sleep(std::time::Duration::from_millis(150));

        let guard = OrphanPidfile::new(&path);
        guard.record(pid);
        assert!(path.exists());

        guard.sweep();
        assert!(!path.exists(), "pidfile should be removed after sweep");

        std::thread::sleep(std::time::Duration::from_millis(300));
        let still_alive = child.try_wait().unwrap().is_none();
        assert!(!still_alive, "orphan should have been killed by sweep");
        let _ = child.kill(); // 兜底，防止断言失败时测试进程泄漏
    }

    #[test]
    fn sweep_refuses_to_kill_on_identity_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("engine.pid");
        let (mut decoy, pid) = spawn_standin();
        std::thread::sleep(std::time::Duration::from_millis(150));

        // 故意写一个不匹配当前进程身份的记录，模拟"pidfile 里记的是早已
        // 退出的旧进程，这个 pid 现在被系统复用给了一个无关进程"。
        std::fs::write(
            &path,
            format!("pid={pid}\nidentity=bogus not-a-real-process 0\n"),
        )
        .unwrap();

        let guard = OrphanPidfile::new(&path);
        guard.sweep();

        std::thread::sleep(std::time::Duration::from_millis(300));
        let still_alive = decoy.try_wait().unwrap().is_none();
        assert!(still_alive, "mismatched identity must NOT be killed");
        let _ = decoy.kill();
    }

    #[test]
    fn sweep_is_a_noop_when_no_pidfile_exists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.pid");
        OrphanPidfile::new(&path).sweep(); // 不 panic 就算过
    }

    #[test]
    fn clear_removes_pidfile() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("engine.pid");
        std::fs::write(&path, "pid=1\nidentity=x\n").unwrap();
        OrphanPidfile::new(&path).clear();
        assert!(!path.exists());
    }
}

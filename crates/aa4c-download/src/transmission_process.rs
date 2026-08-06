//! Transmission 子进程生命周期（D2.4，DOWNLOAD_DESIGN.md §3.6.2）：拉起
//! `transmission-daemon`、装上孤儿进程防护、暴露 RPC 连接所需的
//! 端口/凭据——不含 RPC 客户端本身（D2.3）与任务编排 actor（D2.5，同
//! `lib.rs` 的 `DownloadService` 那一套，但 Transmission 版）。
//!
//! 与 aria2 那条路径（`spawn_and_connect_with_retries`）的关键差异：这里没有
//! "连上就健康检查通过"的重试循环——那需要 D2.3 的 `TransmissionClient` 才能
//! 判断"连上了"，D2.4 范围内先只做"进程起来了、孤儿防护已生效"这一层，
//! D2.3 落地后再在它上面接健康检查+重试（同 aria2 `spawn_and_connect_with_retries`
//! 的形状）。

use std::path::{Path, PathBuf};

use aa4c_types::Result;

use aa4c_engine::{protect_with_job_object, EngineChild, OrphanPidfile, SidecarSpawner};

use crate::transmission_conf;

/// 一个正在运行的 `transmission-daemon` 子进程 + 它的 RPC 连接信息。
pub struct TransmissionProcess {
    child: Box<dyn EngineChild>,
    pidfile: OrphanPidfile,
    pub port: u16,
    pub username: String,
    pub password: String,
}

impl TransmissionProcess {
    /// 拉起一个新的 `transmission-daemon`：
    /// 1. 先清扫上次崩溃可能遗留的孤儿（PID 文件方案，核对身份匹配才杀）；
    /// 2. 重写 `settings.json`（新端口/新凭据）；
    /// 3. 用注入的 `spawner` 拉起进程（`-f --config-dir=...`）；
    /// 4. 装上孤儿进程防护——Windows 走 Job Object（按 PID 事后归属，
    ///    `ProcessSpawner`/`TauriSidecarSpawner` 统一适用）；Linux 经
    ///    `ProcessSpawner` 时已经在 spawn 那一步经 `pre_exec` 装好了
    ///    `PR_SET_PDEATHSIG`（见 `spawner.rs`），这里不用再做什么；macOS
    ///    以及 Tauri 路径下的 Linux 靠这次新写的 PID 文件兜底。
    pub async fn spawn(
        spawner: &dyn SidecarSpawner,
        data_dir: &Path,
        download_dir: &Path,
        opts: &transmission_conf::BtOptions,
    ) -> Result<Self> {
        let pidfile = OrphanPidfile::new(pidfile_path(data_dir));
        pidfile.sweep();

        let conf = transmission_conf::write_settings(&config_dir(data_dir), download_dir, opts)?;
        let args = transmission_conf::spawn_args(&conf.config_dir);
        let child = spawner.spawn(&args, &[]).await?;

        let pid = child.pid();
        pidfile.record(pid);
        protect_with_job_object(pid);

        Ok(Self {
            child,
            pidfile,
            port: conf.port,
            username: conf.username,
            password: conf.password,
        })
    }

    pub fn pid(&self) -> u32 {
        self.child.pid()
    }

    pub fn recent_stdio(&self) -> Vec<String> {
        self.child.recent_stdio()
    }

    /// 关闭。D2.4 范围内先直接强杀——D2.3 接上 RPC 客户端后，调用方应该在
    /// 这之前先做 `session-close` 优雅关闭 + 宽限期（同 aria2 侧
    /// `SHUTDOWN_GRACE` 的先例），这里只负责"进程终止 + 清掉 PID 文件"这
    /// 最后一步。正常关闭路径清掉 PID 文件是对的：不是孤儿，不需要下次
    /// 启动再核对一遍。
    pub async fn kill(self) {
        self.child.kill().await;
        self.pidfile.clear();
    }
}

fn config_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("transmission")
}

fn pidfile_path(data_dir: &Path) -> PathBuf {
    data_dir.join("transmission.pid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use aa4c_engine::ProcessSpawner;

    fn transmission_daemon_path() -> Option<std::path::PathBuf> {
        which_in_path("transmission-daemon")
    }

    /// 手写的最小 `which`：按 PATH 逐个目录找 `<bin><平台可执行后缀>`——
    /// Windows 上文件系统按字面文件名查找，不会像 shell 那样自动给裸名补
    /// `.exe`，漏了这一步会永远找不到（真机 CI 上踩到过：MSI 解包本身其实
    /// 是成功的，是这个 helper 一直在找不带后缀的 `transmission-daemon`）。
    fn which_in_path(bin: &str) -> Option<std::path::PathBuf> {
        let filename = format!("{bin}{}", std::env::consts::EXE_SUFFIX);
        std::env::var_os("PATH").and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|dir| dir.join(&filename))
                .find(|p| p.is_file())
        })
    }

    /// 真实 transmission-daemon 跑起来、能被杀掉、孤儿防护装得上——不测 RPC
    /// 连接（D2.3 才有客户端）。找不到二进制显式 panic，不静默跳过（同
    /// aa4c-download D1 集成测试的既有惯例）。
    #[tokio::test]
    async fn spawns_transmission_daemon_and_kills_it_cleanly() {
        let Some(bin) = transmission_daemon_path() else {
            panic!(
                "transmission-daemon not found in PATH — install it first (macOS: brew install transmission-cli; Linux: apt install transmission-daemon)"
            );
        };
        let spawner = ProcessSpawner::new(bin);
        let dir = tempfile::tempdir().unwrap();
        let download_dir = dir.path().join("downloads");

        let proc = TransmissionProcess::spawn(
            &spawner,
            dir.path(),
            &download_dir,
            &transmission_conf::BtOptions::default(),
        )
        .await
        .expect("transmission-daemon should spawn");
        assert!(proc.pid() > 0);
        assert!(proc.port > 0);
        assert!(!proc.username.is_empty());
        assert!(!proc.password.is_empty());

        // 给它一点时间真正跑起来（前台模式启动很快，但留个缓冲避免测试抖动）。
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        proc.kill().await;
        // kill 之后 pidfile 应该已经被清掉——正常关闭不留痕迹。
        assert!(!pidfile_path(dir.path()).exists());
    }

    /// 上次崩溃遗留的孤儿在下次 spawn 前会被清扫掉（复用 orphan_guard 的
    /// PID 文件逻辑，这里验证 `TransmissionProcess::spawn` 真的调用了它，
    /// 不需要真实 transmission-daemon 二进制——用一个不存在的路径让 spawn
    /// 本身失败也没关系，我们只关心 sweep 是否发生在 spawn 尝试之前）。
    #[tokio::test]
    async fn spawn_sweeps_stale_pidfile_from_previous_crash_before_starting() {
        let dir = tempfile::tempdir().unwrap();
        // 伪造一个"上次崩溃"留下的孤儿：真 spawn 一个占位子进程，写好 PID 文件。
        let mut orphan = if cfg!(windows) {
            std::process::Command::new("cmd")
                .args(["/C", "ping -n 60 127.0.0.1 >NUL"])
                .spawn()
                .unwrap()
        } else {
            std::process::Command::new("sleep")
                .arg("60")
                .spawn()
                .unwrap()
        };
        let orphan_pid = orphan.id();
        std::thread::sleep(std::time::Duration::from_millis(150));
        OrphanPidfile::new(pidfile_path(dir.path())).record(orphan_pid);

        // spawn 本身用一个不存在的二进制，故意失败——我们不关心这次 spawn
        // 成不成功，只关心它触发的 sweep 是否清理了上面伪造的孤儿。
        let spawner = ProcessSpawner::new("aa4c-does-not-exist-binary");
        let download_dir = dir.path().join("downloads");
        let _ = TransmissionProcess::spawn(
            &spawner,
            dir.path(),
            &download_dir,
            &transmission_conf::BtOptions::default(),
        )
        .await;

        std::thread::sleep(std::time::Duration::from_millis(300));
        let still_alive = orphan.try_wait().unwrap().is_none();
        assert!(
            !still_alive,
            "stale orphan should be swept before the new spawn attempt"
        );
        let _ = orphan.kill();
    }
}

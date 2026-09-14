//! 下载中心的插件外壳（R1，`aa4c_plugin::Plugin`）。
//!
//! 此前下载中心是焊在 `Core` 上的：`Core.download: Option<Arc<DownloadService>>`
//! 一个字段、`orchestrate.rs` 里 10 个转发方法、`Settings` 里 12 个字段、
//! 10 个 Tauri Command。但按 PROJECT_VISION 自己的话，AA连接**不是**下载器——
//! 拿掉下载，"我的几台设备连成一片"这件事一点不少。所以它属于插件层。
//!
//! 这一层**不含任何业务逻辑**：`Core` 里那 10 个方法本来就是一行转发
//! （`self.download_service()?.pause(id).await` 之类），这里原样搬过来，只是把
//! 编译期的方法名换成了运行期的 `method` 字符串。校验从类型系统移到
//! `serde_json::from_value`，代价明确写在 `aa4c_plugin::Plugin::invoke` 的文档里。

use std::path::PathBuf;
use std::sync::Arc;

use aa4c_plugin::{Plugin, PluginContext, PluginFuture};
use aa4c_types::{Aa4cError, DownloadOptions, Result, TaskId};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::OnceCell;

use crate::{DownloadLimits, DownloadRequest, DownloadService, DownloadSource, SidecarSpawner};

/// 下载插件。由桌面壳层构造（它才拿得到基于 `tauri-plugin-shell` 的 spawner），
/// 再注册进 `PluginRegistry`。
pub struct DownloadPlugin {
    /// aria2 拉起器——这是"本平台支不支持下载能力"的总闸，与 `bt_spawner` 的
    /// "BT 这一个引擎可不可用"是两回事（沿用 `CoreConfig` 上原有的两段式语义）。
    spawner: Arc<dyn SidecarSpawner>,
    bt_spawner: Option<Arc<dyn SidecarSpawner>>,
    /// `start()` 之后才有。用 `OnceCell` 而不是 `Mutex<Option<_>>`：只写一次，
    /// 读多写少，且读路径（每次 `invoke`）不该去抢锁。
    /// 见 `ArchivePlugin::running` 的文档：裸 `OnceCell` 的 clone 是独立格子，
    /// `start()` 往它里面写等于白写。
    service: Arc<OnceCell<Arc<DownloadService>>>,
}

impl DownloadPlugin {
    pub fn new(
        spawner: Arc<dyn SidecarSpawner>,
        bt_spawner: Option<Arc<dyn SidecarSpawner>>,
    ) -> Self {
        Self {
            spawner,
            bt_spawner,
            service: Arc::new(OnceCell::new()),
        }
    }

    /// 未启动 / 启动失败时报 `Unavailable`——与 `Core::download_service()` 此前
    /// 对 `None` 的处理逐字一致，前端看到的错误不变。
    fn service(&self) -> Result<&Arc<DownloadService>> {
        self.service
            .get()
            .ok_or_else(|| Aa4cError::Unavailable("download center is not running".into()))
    }
}

/// 插件自己的设置项。此前是 `aa4c_types::Settings` 里的 12 个字段——占那个
/// 28 字段结构体的近一半，也是设置页涨到 5 个 tab 的主因。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct DownloadSettings {
    pub download_dir: Option<String>,
    pub speed_limit_kbps: Option<u32>,
    pub upload_limit_kbps: Option<u32>,
    pub concurrency: Option<u32>,
    pub max_connections_per_file: Option<u32>,
    pub user_agent: Option<String>,
    pub proxy: Option<String>,
    pub proxy_bypass: Option<String>,
    pub bt_trackers: Option<String>,
    pub bt_ratio_limit: Option<f64>,
    pub bt_idle_seeding_limit_minutes: Option<u32>,
    pub resume_on_start: bool,
}

impl DownloadSettings {
    fn limits(&self) -> DownloadLimits {
        DownloadLimits {
            speed_limit_kbps: self.speed_limit_kbps,
            upload_limit_kbps: self.upload_limit_kbps,
            concurrency: self.concurrency,
            max_connections_per_file: self.max_connections_per_file,
            user_agent: self.user_agent.clone(),
            proxy: self.proxy.clone(),
            proxy_bypass: self.proxy_bypass.clone(),
            bt_ratio_limit: self.bt_ratio_limit,
            bt_idle_seeding_limit_minutes: self.bt_idle_seeding_limit_minutes,
            bt_trackers: self.bt_trackers.clone(),
        }
    }
}

/// 各方法的入参。**故意不用 `#[serde(untagged)]` 合成一个枚举**：untagged 会
/// 按顺序试每个变体，于是「方法名对、参数形状错」会落到匹配失败的路径上，报出
/// 「未知方法 pause」——方法明明存在，错误信息是骗人的。按方法各自反序列化，
/// 报出来的就是这个方法缺哪个字段。
mod args {
    use super::*;

    #[derive(Debug, Deserialize)]
    pub(super) struct Add {
        pub url: String,
        #[serde(default)]
        pub options: Option<DownloadOptions>,
    }

    #[derive(Debug, Deserialize)]
    pub(super) struct Torrent {
        pub path: PathBuf,
        #[serde(default)]
        pub options: Option<DownloadOptions>,
    }

    #[derive(Debug, Deserialize)]
    pub(super) struct Cancel {
        pub id: TaskId,
        pub delete_local: bool,
    }

    #[derive(Debug, Deserialize)]
    pub(super) struct Id {
        pub id: TaskId,
    }
}

/// 反序列化一个方法的入参，失败时把**方法名**带进错误里。
fn parse<T: serde::de::DeserializeOwned>(method: &str, payload: Value) -> Result<T> {
    serde_json::from_value(payload)
        .map_err(|e| Aa4cError::Protocol(format!("download.{method}: bad arguments: {e}")))
}

impl Plugin for DownloadPlugin {
    fn id(&self) -> &'static str {
        "download"
    }

    fn display_name(&self) -> &'static str {
        "下载"
    }

    fn start(&self, ctx: PluginContext) -> PluginFuture<()> {
        let spawner = self.spawner.clone();
        let bt_spawner = self.bt_spawner.clone();
        let cell = self.service.clone();
        Box::pin(async move {
            let settings: DownloadSettings =
                serde_json::from_value(ctx.settings.clone()).unwrap_or_default();
            let download_dir = settings
                .download_dir
                .clone()
                .map(PathBuf::from)
                .unwrap_or_else(default_download_dir);

            // 起不来不报错，只降级——`DownloadService::start` 内部已经处理引擎
            // 拉不起来的情况（返回的 service 里 `cmd_tx` 为空），这跟"本平台没有
            // 下载能力"是两种不同的不可用，沿用既有语义。
            let service = DownloadService::start(
                spawner,
                bt_spawner,
                ctx.store.clone(),
                ctx.events.clone(),
                ctx.data_dir.clone(),
                download_dir,
                settings.limits(),
            )
            .await;

            // 启动时自动继续未完成的下载（对标 Motrix 的
            // `resume-all-when-app-launched`，默认关闭）。必须在 `start` 之后——
            // 那里面跑完了首轮对账，此刻库里的状态才是引擎的真实状态。后台 spawn
            // 而不是 await：任务多时逐个发 RPC 不该拖慢启动。
            if settings.resume_on_start {
                let svc = service.clone();
                tokio::spawn(async move {
                    let n = svc.resume_all().await;
                    if n > 0 {
                        tracing::info!(count = n, "resumed unfinished downloads on startup");
                    }
                });
            }

            let _ = cell.set(service);
            Ok(())
        })
    }

    fn shutdown(&self) -> PluginFuture<()> {
        let service = self.service.get().cloned();
        Box::pin(async move {
            if let Some(svc) = service {
                svc.shutdown().await;
            }
            Ok(())
        })
    }

    fn invoke(&self, method: String, payload: Value) -> PluginFuture<Value> {
        let service = self.service().cloned();
        Box::pin(async move {
            let svc = service?;
            let m = method.as_str();
            let out = match m {
                "add" => {
                    let a: args::Add = parse(m, payload)?;
                    json!(
                        svc.add(DownloadRequest {
                            source: DownloadSource::Uri(a.url),
                            options: a.options.unwrap_or_default(),
                        })
                        .await?
                    )
                }
                "add_torrent_file" => {
                    let a: args::Torrent = parse(m, payload)?;
                    json!(
                        svc.add(DownloadRequest {
                            source: DownloadSource::TorrentFile(a.path),
                            options: a.options.unwrap_or_default(),
                        })
                        .await?
                    )
                }
                "pause" => json!(svc.pause(parse::<args::Id>(m, payload)?.id).await?),
                "resume" => json!(svc.resume(parse::<args::Id>(m, payload)?.id).await?),
                "cancel" => {
                    let a: args::Cancel = parse(m, payload)?;
                    json!(svc.cancel(a.id, a.delete_local).await?)
                }
                "retry" => json!(svc.retry(parse::<args::Id>(m, payload)?.id).await?),
                "list" => json!(svc.list().await?),
                "pause_all" => json!(svc.pause_all().await),
                "resume_all" => json!(svc.resume_all().await),
                "clear_completed" => json!(svc.clear_completed().await?),
                other => {
                    return Err(Aa4cError::Protocol(format!(
                        "download: unknown method {other}"
                    )))
                }
            };
            Ok(out)
        })
    }

    fn settings_schema(&self) -> Value {
        json!({
            "title": "下载",
            "note": "改动需要重启应用才生效——这些值会写进引擎启动时生成的配置文件，不做热更新。",
            "fields": [
                { "key": "download_dir", "type": "dir", "label": "下载目录",
                  "hint": "必须在「接收文件保存到」目录之外——放进去会被自动索引并分享给全部完全信任设备" },
                { "key": "speed_limit_kbps", "type": "u32?", "label": "下载限速（KB/s）", "hint": "留空 = 不限速" },
                { "key": "upload_limit_kbps", "type": "u32?", "label": "上传限速（KB/s）", "hint": "留空 = 不限速" },
                { "key": "concurrency", "type": "u32?", "label": "同时下载数", "hint": "留空 = 用引擎默认值" },
                { "key": "max_connections_per_file", "type": "u32?", "label": "单文件最大连接数", "hint": "1–16，别的下载器说的「多线程下载」" },
                { "key": "user_agent", "type": "string?", "label": "User-Agent" },
                { "key": "proxy", "type": "string?", "label": "代理" },
                { "key": "proxy_bypass", "type": "string?", "label": "不走代理的地址" },
                { "key": "bt_trackers", "type": "text?", "label": "追加 tracker", "hint": "一行一个" },
                { "key": "bt_ratio_limit", "type": "f64?", "label": "分享率上限", "hint": "做种到这个分享率就停，留空 = 不限" },
                { "key": "bt_idle_seeding_limit_minutes", "type": "u32?", "label": "空闲做种超时（分钟）" },
                { "key": "resume_on_start", "type": "bool", "label": "启动时继续未完成的下载" }
            ]
        })
    }
}

/// 平台默认下载目录。与 `aa4c_core::settings::default_download_dir` 逐字一致。
fn default_download_dir() -> PathBuf {
    dirs::download_dir().unwrap_or_else(std::env::temp_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 没 `start()` 过就调用，报 `Unavailable`——与此前 `Core::download_service()`
    /// 对 `download: None` 的处理一致，前端看到的错误不变。
    #[tokio::test]
    async fn invoking_before_start_is_unavailable() {
        let plugin = DownloadPlugin::new(Arc::new(crate::ProcessSpawner::new("nonexistent")), None);
        let err = plugin
            .invoke("list".into(), json!({}))
            .await
            .expect_err("should be unavailable");
        assert!(matches!(err, Aa4cError::Unavailable(_)), "got {err:?}");
    }

    /// 方法名对、参数形状错时，报的必须是**参数**问题，而不是「未知方法」。
    ///
    /// 这条是冲着 `#[serde(untagged)]` 那个写法来的：untagged 按顺序试变体，
    /// 于是 `("pause", 形状像 add 的参数)` 会两头都不匹配，落到兜底分支报
    /// 「unknown method pause」——方法明明存在。改成按方法各自反序列化之后，
    /// 错误信息才指向真正出问题的地方。
    #[tokio::test]
    async fn a_known_method_with_bad_arguments_does_not_report_unknown_method() {
        let plugin = DownloadPlugin::new(Arc::new(crate::ProcessSpawner::new("nonexistent")), None);
        // 先让它过掉「没 start」那道门，直接测参数解析这一层。
        let err = parse::<args::Id>("pause", json!({ "url": "http://x" })).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("download.pause"), "错误要指名方法：{msg}");
        assert!(!msg.contains("unknown method"), "不该说方法不存在：{msg}");
        drop(plugin);
    }

    /// 12 个设置项当初散在 `aa4c_types::Settings` 里；这里确认插件能原样吃回去。
    #[test]
    fn settings_round_trip_through_json() {
        let raw = json!({
            "download_dir": "/tmp/dl",
            "concurrency": 3,
            "bt_ratio_limit": 1.5,
            "resume_on_start": true
        });
        let s: DownloadSettings = serde_json::from_value(raw).unwrap();
        assert_eq!(s.download_dir.as_deref(), Some("/tmp/dl"));
        assert_eq!(s.limits().concurrency, Some(3));
        assert_eq!(s.limits().bt_ratio_limit, Some(1.5));
        assert!(s.resume_on_start);
    }

    /// 缺字段一律回落默认值，不报错——设置项是渐进填的，插件第一次启动时
    /// `ctx.settings` 可能整个是 `null`。
    #[test]
    fn missing_settings_fall_back_to_defaults_rather_than_failing() {
        let s: DownloadSettings = serde_json::from_value(Value::Null).unwrap_or_default();
        assert!(s.download_dir.is_none());
        assert!(!s.resume_on_start);
    }
}

//! 应用设置（API_DESIGN.md §9 / DATABASE_SCHEMA.md §2.4）。
//!
//! 持久化为 settings 表的若干 KV（值 JSON 编码）；此处只定义前后端共享的
//! 聚合视图，键名与默认值的解析归 aa4c-core 负责。

use serde::{Deserialize, Serialize};

// `Eq` 去掉了：`bt_ratio_limit: Option<f64>` 加进来后整个结构体不能再自动派生
// `Eq`（`f64` 只有 `PartialEq`，NaN 不满足自反性）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// 本机设备名（默认 hostname）。
    pub device_name: String,
    /// 默认接收目录（绝对路径字符串，默认 ~/Downloads/AA4C）。
    pub save_dir: String,
    /// 已配对设备来文件是否免确认（默认 false）。
    pub auto_accept_from_trusted: bool,
    /// 监听端口（默认 42420）。
    pub listen_port: u16,
    /// 自建 `aa4c-server` 地址（`aa4c://host:port#指纹`），默认未配置
    /// （CONNECT_DESIGN.md §8，里程碑 C2）。
    pub server_url: Option<String>,
    /// 远程连接总开关，默认 **关闭**——不配置、不打开就完全不出网（CONNECT_DESIGN.md §8）。
    pub enable_remote: bool,
    /// 下载目录（默认系统下载目录，如 `~/Downloads`），必须在 `save_dir` 子树之外——
    /// 落进 `save_dir` 会被 Inbox 自动索引、分享给全部完全信任设备（DOWNLOAD_DESIGN.md
    /// §5/§7，里程碑 D1）。
    pub download_dir: String,
    /// 下载限速（KB/s），`None` 或 0 = 不限速。写进每次启动重新生成的引擎配置文件，
    /// 下次启动生效，不做热更新（DOWNLOAD_DESIGN.md §9，里程碑 D3）。
    pub download_speed_limit_kbps: Option<u32>,
    /// 并发下载数，`None` = 用各引擎自己的默认值（aria2 默认 5）。同上，重启生效。
    pub download_concurrency: Option<u32>,
    /// BT 分享率上限，`None` = 不限。对应 Transmission `ratio-limit`（配置文件键名，
    /// 与 RPC session-set 的 `seedRatioLimit` 不是同一个名字，见 DOWNLOAD_DESIGN.md §9）。
    pub bt_ratio_limit: Option<f64>,
    /// BT 空闲做种超时（分钟），`None` = 不限。Transmission 没有"总做种时长"概念，
    /// 这是"多久没有上传活动就停止做种"（`idle-seeding-limit`，DOWNLOAD_DESIGN.md §9）。
    pub bt_idle_seeding_limit_minutes: Option<u32>,
    /// 归档根目录（默认系统文档目录下的 `AA4C归档`），必须与 `save_dir`/`download_dir`
    /// 子树互不嵌套（同 `download_dir` 的既有隔离原则，ARCHIVE_DESIGN.md §2.5）。
    pub archive_root: String,
    /// 自动归档总闸（下载完成后是否跑规则引擎），默认开启——真正的保守闸门在每条
    /// 规则各自的 `enabled`（默认停用），见 ARCHIVE_DESIGN.md §2.3。
    pub archive_auto_enabled: bool,
    /// 模型文件目录（默认 `<归档根>/模型`——与内置"模型"归档规则的目标目录故意
    /// 同址：下载 GGUF → 自动归档进模型目录 → 模型库立即可见，ARCHIVE_DESIGN.md
    /// §3.5）。
    pub ai_models_dir: String,
    /// 当前选定的对话模型文件路径，`None` = 未配置（AI 能力整体 `Unavailable`）。
    pub ai_chat_model: Option<String>,
    /// 当前选定的嵌入模型文件路径，`None` = 未配置。
    pub ai_embedding_model: Option<String>,
    /// AI 引擎空闲多久后自动退出释放内存（分钟），默认 10（ARCHIVE_DESIGN.md §3.3）。
    pub ai_idle_timeout_minutes: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_json_is_camel_case() {
        let s = Settings {
            device_name: "Huo 的 MacBook".into(),
            save_dir: "/Users/huo/Downloads/AA4C".into(),
            auto_accept_from_trusted: false,
            listen_port: 42420,
            server_url: Some("aa4c://example.com:42420#abcd1234abcd1234".into()),
            enable_remote: true,
            download_dir: "/Users/huo/Downloads".into(),
            download_speed_limit_kbps: Some(500),
            download_concurrency: Some(3),
            bt_ratio_limit: Some(2.0),
            bt_idle_seeding_limit_minutes: Some(30),
            archive_root: "/Users/huo/Documents/AA4C归档".into(),
            archive_auto_enabled: true,
            ai_models_dir: "/Users/huo/Documents/AA4C归档/模型".into(),
            ai_chat_model: Some("/Users/huo/Documents/AA4C归档/模型/qwen3-4b.gguf".into()),
            ai_embedding_model: None,
            ai_idle_timeout_minutes: 10,
        };
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["deviceName"], "Huo 的 MacBook");
        assert_eq!(json["autoAcceptFromTrusted"], false);
        assert_eq!(json["listenPort"], 42420);
        assert_eq!(
            json["serverUrl"],
            "aa4c://example.com:42420#abcd1234abcd1234"
        );
        assert_eq!(json["enableRemote"], true);
        assert_eq!(json["downloadDir"], "/Users/huo/Downloads");
        assert_eq!(json["downloadSpeedLimitKbps"], 500);
        assert_eq!(json["downloadConcurrency"], 3);
        assert_eq!(json["btRatioLimit"], 2.0);
        assert_eq!(json["btIdleSeedingLimitMinutes"], 30);
        assert_eq!(json["archiveRoot"], "/Users/huo/Documents/AA4C归档");
        assert_eq!(json["archiveAutoEnabled"], true);
        assert_eq!(json["aiModelsDir"], "/Users/huo/Documents/AA4C归档/模型");
        assert_eq!(
            json["aiChatModel"],
            "/Users/huo/Documents/AA4C归档/模型/qwen3-4b.gguf"
        );
        assert_eq!(json["aiEmbeddingModel"], serde_json::Value::Null);
        assert_eq!(json["aiIdleTimeoutMinutes"], 10);
        let back: Settings = serde_json::from_value(json).unwrap();
        assert_eq!(back, s);
    }
}

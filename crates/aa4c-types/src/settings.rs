//! 应用设置（API_DESIGN.md §9 / DATABASE_SCHEMA.md §2.4）。
//!
//! 持久化为 settings 表的若干 KV（值 JSON 编码）；此处只定义前后端共享的
//! 聚合视图，键名与默认值的解析归 aa4c-core 负责。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
        let back: Settings = serde_json::from_value(json).unwrap();
        assert_eq!(back, s);
    }
}

//! 应用设置（API_DESIGN.md §9 / DATABASE_SCHEMA.md §2.4）。
//!
//! 持久化为 settings 表的若干 KV（值 JSON 编码）；此处只定义前后端共享的
//! 聚合视图，键名与默认值的解析归 aa4c-core 负责。
//!
//! **只装连接层与能力层自己的设置**（V0.8「Focus」F1.3）。插件的设置走 [`Settings::plugins`]
//! 这个不透明口袋——核心不认识它们的字段名，那正是 F1 划出插件边界时要买的东西。

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
    /// 自动在路由器上开端口（UPnP IGD），默认 **开**——但**只在 `enable_remote` 打开时
    /// 才生效**（TRUST_DESIGN.md §6.2，里程碑 R3）。
    ///
    /// 两层闸的分工要说清：`enable_remote` 默认关闭，「不配置、不打开就完全不出网」这条
    /// 默认安全的姿态由它保证；一旦用户主动打开了远程连接，他要的就是「能被连上」，这时
    /// 再要求他找第二个开关才肯打洞是反的——所以这一项在那个前提下默认开（与 Transmission
    /// 自己 `port-forwarding-enabled: true` 的默认一致）。留这个开关是因为它确实会**在用户
    /// 的路由器上开一个端口**，有人不接受这件事，界面上必须写明白。
    pub enable_port_mapping: bool,
    /// 在本机内置一个 `aa4c-server`，让这台常开的设备兼任汇合点，默认 **关闭**
    /// （TRUST_DESIGN.md §6.3，里程碑 R4）。
    ///
    /// 门槛从「要有 VPS」降到「家里有台常开设备」。**但要诚实说清前提**：你仍然需要一个
    /// **稳定入口**才能被找到——DDNS 或相对固定的地址。这是「零第三方」的真实边界：不需要
    /// 服务商，但需要你自己有个能被找到的落脚点。
    pub enable_local_server: bool,
    /// 内置服务器监听端口，默认 42421。
    ///
    /// **不能与 `listen_port`（传输端口，默认 42420）相同**：传输层已经占了那个号的
    /// TCP 与 UDP，内置服务器同样要 TCP（信令/中继）+ UDP（反射端点）。
    pub local_server_port: u16,
    /// 别的设备该用哪个主机名/地址找到这台内置服务器（DDNS 域名或固定 IP），可空。
    ///
    /// 服务器自己不知道对外可见的域名——这部分只能由用户提供。留空时界面会退而用探测到的
    /// 公网地址（UPnP 映射拿到的）或本机局域网地址，并**如实标明**那是什么。
    pub local_server_host: Option<String>,
    /// 各插件自己的设置，按插件 id 索引（V0.8「Focus」F1.3）。
    ///
    /// **核心不解释里面的内容。** 此前这个结构体有 28 个字段，其中 18 个属于下载
    /// （12 个）与归档/AI（6 个）——它们是设置页涨到 5 个标签页、903 行的直接原因，
    /// 也让「AA4C 不是下载器」这句话在类型层面就不成立。
    ///
    /// 现在每个插件用 `Plugin::settings_schema()` 声明自己有哪些字段，前端用一个通用
    /// 渲染器画表单；这里只是个不透明的 JSON 口袋。落库时一个插件一条记录，
    /// 键名 `plugin.<id>`。
    #[serde(default)]
    pub plugins: serde_json::Map<String, serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_json_is_camel_case() {
        let mut plugins = serde_json::Map::new();
        plugins.insert(
            "download".into(),
            serde_json::json!({ "download_dir": "/Users/huo/Downloads", "concurrency": 3 }),
        );

        let s = Settings {
            device_name: "Huo 的 MacBook".into(),
            save_dir: "/Users/huo/Downloads/AA4C".into(),
            auto_accept_from_trusted: false,
            listen_port: 42420,
            server_url: Some("aa4c://example.com:42420#abcd1234abcd1234".into()),
            enable_remote: true,
            enable_port_mapping: true,
            enable_local_server: true,
            local_server_port: 42421,
            local_server_host: Some("home.example.com".into()),
            plugins,
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
        assert_eq!(json["enablePortMapping"], true);
        assert_eq!(json["enableLocalServer"], true);
        assert_eq!(json["localServerPort"], 42421);
        assert_eq!(json["localServerHost"], "home.example.com");

        // 插件那一格**原样透传**，核心不碰里面的键名——所以里面是下划线风格，
        // 与外层的 camelCase 不同。那是插件自己的约定，核心无权改写。
        assert_eq!(
            json["plugins"]["download"]["download_dir"],
            "/Users/huo/Downloads"
        );
        assert_eq!(json["plugins"]["download"]["concurrency"], 3);

        let back: Settings = serde_json::from_value(json).unwrap();
        assert_eq!(back, s);
    }

    /// 老版本存下来的 JSON 里没有 `plugins` 字段，反序列化不能炸——
    /// `#[serde(default)]` 保证它回落成空表（升级路径，见 `Settings::plugins` 文档）。
    #[test]
    fn settings_without_a_plugins_field_still_deserialize() {
        let json = serde_json::json!({
            "deviceName": "旧版本",
            "saveDir": "/tmp",
            "autoAcceptFromTrusted": false,
            "listenPort": 42420,
            "serverUrl": null,
            "enableRemote": false,
            "enablePortMapping": true,
            "enableLocalServer": false,
            "localServerPort": 42421,
            "localServerHost": null
        });
        let s: Settings = serde_json::from_value(json).unwrap();
        assert!(s.plugins.is_empty());
    }
}

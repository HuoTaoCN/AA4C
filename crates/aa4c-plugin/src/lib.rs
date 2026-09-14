//! 插件边界（ARCHITECTURE.md 原则 3 / AGENTS.md「必须：插件化」）。
//!
//! 这个抽象被三份文档承诺了三个版本，但在 F1 之前**从来不存在**——下载中心与
//! 归档/AI 是直接焊进 `Core` 的：`Core` 上挂着 `download`/`ai`/`suggest`/`kb`
//! 四个具体字段，`orchestrate.rs` 里 1/3 的编排方法、`Settings` 里 18/28 个字段、
//! 60 个 Tauri 命令里的 27 个都属于它们。于是「设备连接平台」这个定位在代码里
//! 被稀释成了一半，UI 也跟着长成了一个功能宫格。
//!
//! # 边界划在哪
//!
//! **连接层与能力层留在核心**（身份 / 发现 / 信任 / 连接阶梯 / 传输 / 同步 / 分享）——
//! 它们是「设备连成一片」这件事本身。**其余一律是插件**：下载中心、归档与 AI、知识库。
//! 判据不是「重不重要」，而是「拿掉它之后，AA4C 还是不是 AA4C」。
//!
//! # 为什么是编译期 trait，而不是动态加载
//!
//! Rust 没有稳定 ABI，`dylib` 插件跨编译器版本就会炸。第三方插件是 V1.0 的议题，
//! 那时的载体应该是进程外 RPC 或脚本（DOWNLOAD_DESIGN.md §10 预留的 Lua 属于后者），
//! 不是 `dlopen`。现阶段一个编译期 trait + 一个通用调用入口就够，而且它立刻能把
//! 核心与插件的依赖方向钉死——这才是本轮真正要买的东西。
//!
//! # 为什么是独立 crate
//!
//! 插件要 `impl Plugin`，就得依赖 trait 所在的 crate。而本轮的目标恰恰是让
//! **核心不依赖任何插件**——trait 放在 `aa4c-core` 里，`aa4c-download` 为了实现它
//! 反过来依赖 `aa4c-core`，就成了循环。抽成这个只有一个文件的 crate，
//! 依赖方向变成 `core → plugin ← 各插件`，两边都干净。
//! `aa4c_core::plugin` 是对它的 re-export，既有路径不变。
//!
//! # 异步写法
//!
//! trait 方法返回装箱 future（[`PluginFuture`]），沿用本仓库既有惯例
//! （`aa4c_transfer::ResolveFuture` / `RelayDialFuture` / `PunchFuture`），
//! **不引入 `async_trait` 依赖**。AFIT 在 `dyn Trait` 上还不可用，而插件注册表
//! 必须是 `Vec<Arc<dyn Plugin>>`。

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use aa4c_store::Store;
use aa4c_types::{Aa4cError, CoreEvent, Result};
use serde_json::Value;
use tokio::sync::broadcast;

/// 插件异步方法的返回类型。见模块文档「异步写法」。
pub type PluginFuture<T> = Pin<Box<dyn Future<Output = Result<T>> + Send>>;

/// 启动时交给插件的一切。插件**只能**通过它接触外界——拿不到 `Core`、拿不到
/// `TransferService`、拿不到 `Identity`，依赖方向因此是单向的（AGENTS.md 架构规则）。
#[derive(Clone)]
pub struct PluginContext {
    /// 与核心共用的那一条 SQLite 连接（`Store` 是廉价克隆句柄，内部单线程）。
    ///
    /// **不给插件独立数据库**：下载任务要能被归档规则读到、归档条目要能进同步索引，
    /// 拆库就得自己造跨库事务。插件用 [`Store::with_conn`] 执行自己的 SQL；
    /// 新建的表按 `<插件 id>_` 前缀命名（既有表保留原名，见下）。
    ///
    /// **迁移不随插件走**：`aa4c-store` 的迁移是一条 `PRAGMA user_version` 线性链，
    /// 把 008–011 摘出去意味着重编号，而重编号要重建表——迁移期间外键是关着的，
    /// `DROP TABLE` 的隐式 DELETE 会顺着级联删光子表（本项目已经栽过一次，见
    /// CHANGELOG `0.7.0-preview` 之前那轮「数据库迁移重建表时会顺着外键级联删光子表
    /// 数据」）。既有表因此原地不动、原名保留，搬的只是 CRUD 方法。
    pub store: Store,
    /// 事件总线发送端。插件只应发 [`CoreEvent::Plugin`]，不应伪造核心事件。
    pub events: broadcast::Sender<CoreEvent>,
    /// 本插件的私有数据目录（`<data_dir>/plugins/<id>/`），启动前已建好。
    pub data_dir: PathBuf,
    /// 本插件的设置项，形状由插件自己的 [`Plugin::settings_schema`] 决定。
    pub settings: Value,
}

/// 一个可插拔的高级能力。
///
/// 实现者住在自己的 crate 里（`aa4c-download` / `aa4c-plugin-archive`），
/// **核心不依赖它们**——是桌面端在装配时把实现塞进 [`PluginRegistry`]。
pub trait Plugin: Send + Sync + 'static {
    /// 稳定标识符，用于路由调用、命名设置项与数据目录。全小写、无空格。
    fn id(&self) -> &'static str;

    /// 面向用户的名字（进「更多」分区的导航项）。
    fn display_name(&self) -> &'static str;

    /// 拉起。失败**不应阻断应用启动**——调用方会记一条 warn 然后继续，
    /// 同 `Core` 对 `download`/`ai` 既有的「能力不可用」降级语义。
    fn start(&self, ctx: PluginContext) -> PluginFuture<()>;

    /// 停机。必须幂等：`Core::shutdown` 可能被调用多次。
    fn shutdown(&self) -> PluginFuture<()>;

    /// 通用调用入口。Tauri 的 `plugin_invoke(id, method, payload)` 转发到这里。
    ///
    /// 用一个通用入口而不是每个插件注册一批类型化 Command：Tauri 的
    /// `invoke_handler!` 是编译期展开的，按插件分组会让 `lib.rs` 长出一堆 `#[cfg]`；
    /// 通用入口一次到位，也给 V1.0 的第三方插件留了真门。代价是参数校验从编译期
    /// 移到运行期——由插件自己在 `invoke` 里反序列化时保证。
    fn invoke(&self, method: String, payload: Value) -> PluginFuture<Value>;

    /// 本插件的设置项被改了。
    ///
    /// 默认什么都不做——绝大多数设置是启动时读一次就够了。实现它的理由只有一个：
    /// **有些改动必须立刻生效，不能等下次重启**。归档插件换模型文件就是这种——
    /// `AiService::set_model` 会把正在跑的旧进程顺手停掉，下一次请求用新模型懒启动
    /// （ARCHIVE_DESIGN.md §3.3）。没有这个回调，那段逻辑就只能继续赖在 `Core` 的
    /// `update_settings` 里，而那正是本轮要拆掉的耦合。
    fn on_settings_changed(&self, _settings: Value) -> PluginFuture<()> {
        Box::pin(async { Ok(()) })
    }

    /// 设置项 schema，供前端的通用设置渲染器画表单。
    ///
    /// 目的是让 `SettingsPage` 不再为每个插件手写一段表单——那正是它涨到 903 行、
    /// 5 个 tab、28 个字段的原因（其中 18 个属于下载与 AI）。
    fn settings_schema(&self) -> Value;
}

/// 本次构建装配了哪些插件。
///
/// 顺序即启动顺序与「更多」分区里的展示顺序。
#[derive(Default, Clone)]
pub struct PluginRegistry {
    plugins: Vec<Arc<dyn Plugin>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册一个插件。**重复 id 直接 panic**：这是装配期的编程错误，不是运行期状况,
    /// 早炸比让两个插件互相覆盖对方的设置与数据目录好。
    pub fn register(&mut self, plugin: Arc<dyn Plugin>) -> &mut Self {
        let id = plugin.id();
        assert!(
            !id.is_empty() && id.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
            "plugin id must be lowercase ascii: {id:?}"
        );
        assert!(
            self.get(id).is_none(),
            "duplicate plugin id registered: {id:?}"
        );
        self.plugins.push(plugin);
        self
    }

    pub fn get(&self, id: &str) -> Option<&Arc<dyn Plugin>> {
        self.plugins.iter().find(|p| p.id() == id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Arc<dyn Plugin>> {
        self.plugins.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// 本次构建装了哪些插件的 id。设置层据此只读真的装了的那几份。
    pub fn ids(&self) -> Vec<&'static str> {
        self.plugins.iter().map(|p| p.id()).collect()
    }

    /// 路由一次调用。未知 id 报 `Unavailable` 而不是 `Protocol`——对前端来说
    /// 「这个构建没装这个插件」与「这个能力不可用」是同一件事，同 `Core` 对
    /// `download`/`ai` 为 `None` 时的既有语义。
    pub async fn invoke(&self, id: &str, method: String, payload: Value) -> Result<Value> {
        let plugin = self
            .get(id)
            .ok_or_else(|| Aa4cError::Unavailable(format!("plugin not installed: {id}")))?;
        plugin.invoke(method, payload).await
    }

    /// 逐个拉起。单个插件失败只记 warn——一个下载引擎起不来不该让设备连不上。
    pub async fn start_all(&self, base: &PluginContextBuilder) {
        for plugin in &self.plugins {
            let id = plugin.id();
            match base.build(id) {
                Ok(ctx) => {
                    if let Err(e) = plugin.start(ctx).await {
                        tracing::warn!(plugin = id, error = %e, "plugin failed to start");
                    } else {
                        tracing::info!(plugin = id, "plugin started");
                    }
                }
                Err(e) => tracing::warn!(plugin = id, error = %e, "plugin context unavailable"),
            }
        }
    }

    /// 广播一次设置变更。失败只记 warn——一个插件没接住新设置，不该让
    /// 「保存设置」这个动作对用户报错。
    pub async fn notify_settings(&self, settings: &serde_json::Map<String, Value>) {
        for plugin in &self.plugins {
            let own = settings.get(plugin.id()).cloned().unwrap_or(Value::Null);
            if let Err(e) = plugin.on_settings_changed(own).await {
                tracing::warn!(plugin = plugin.id(), error = %e, "plugin rejected new settings");
            }
        }
    }

    /// 逐个停机，失败只记 warn（停机路径不该因为一个插件而卡住）。
    pub async fn shutdown_all(&self) {
        for plugin in &self.plugins {
            if let Err(e) = plugin.shutdown().await {
                tracing::warn!(plugin = plugin.id(), error = %e, "plugin shutdown failed");
            }
        }
    }
}

/// 造 [`PluginContext`] 的材料。每个插件拿到的只有 `data_dir` 与 `settings` 不同。
pub struct PluginContextBuilder {
    pub store: Store,
    pub events: broadcast::Sender<CoreEvent>,
    /// 应用数据根目录；插件目录是它下面的 `plugins/<id>/`。
    pub data_dir: PathBuf,
    /// 全部插件的设置，按 id 索引。
    pub settings: serde_json::Map<String, Value>,
}

impl PluginContextBuilder {
    fn build(&self, id: &str) -> Result<PluginContext> {
        let data_dir = self.data_dir.join("plugins").join(id);
        std::fs::create_dir_all(&data_dir)?;
        Ok(PluginContext {
            store: self.store.clone(),
            events: self.events.clone(),
            data_dir,
            settings: self.settings.get(id).cloned().unwrap_or(Value::Null),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Stub(&'static str);

    impl Plugin for Stub {
        fn id(&self) -> &'static str {
            self.0
        }
        fn display_name(&self) -> &'static str {
            "测试插件"
        }
        fn start(&self, _ctx: PluginContext) -> PluginFuture<()> {
            Box::pin(async { Ok(()) })
        }
        fn shutdown(&self) -> PluginFuture<()> {
            Box::pin(async { Ok(()) })
        }
        fn invoke(&self, method: String, payload: Value) -> PluginFuture<Value> {
            Box::pin(async move { Ok(serde_json::json!({ "method": method, "echo": payload })) })
        }
        fn settings_schema(&self) -> Value {
            Value::Null
        }
    }

    #[tokio::test]
    async fn invoke_routes_to_the_named_plugin() {
        let mut reg = PluginRegistry::new();
        reg.register(Arc::new(Stub("download")))
            .register(Arc::new(Stub("archive")));

        let out = reg
            .invoke("archive", "list_rules".into(), Value::Null)
            .await
            .unwrap();
        assert_eq!(out["method"], "list_rules");
    }

    /// 没装的插件报 `Unavailable`，不是 `Protocol`——对前端来说「这个构建没装它」
    /// 与「这个能力不可用」是同一件事，与 `Core` 对 `download`/`ai` 为 `None` 时
    /// 的既有语义一致。
    #[tokio::test]
    async fn invoking_a_plugin_that_is_not_installed_is_unavailable_not_protocol() {
        let reg = PluginRegistry::new();
        let err = reg
            .invoke("download", "list".into(), Value::Null)
            .await
            .unwrap_err();
        assert!(
            matches!(err, Aa4cError::Unavailable(_)),
            "expected Unavailable, got {err:?}"
        );
    }

    #[test]
    #[should_panic(expected = "duplicate plugin id")]
    fn duplicate_ids_are_a_wiring_bug_and_panic_early() {
        let mut reg = PluginRegistry::new();
        reg.register(Arc::new(Stub("download")));
        reg.register(Arc::new(Stub("download")));
    }
}

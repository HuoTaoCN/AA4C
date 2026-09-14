//! 核心事件（API_DESIGN.md §3.2）。事件总线与 Tauri 事件共用。

use serde::{Deserialize, Serialize};

use crate::{DeviceId, DeviceInfo, KbAnswerSource, TaskId, TransferTask};

/// 一次连接实际走的档位（CONNECT_DESIGN.md §2 连接阶梯）。
///
/// # 为什么要分这么细（V0.8「Focus」F2）
///
/// 此前这里只有 `Direct` / `Punch` / `Relay` 三档，注释写着「局域网直连与公网直连
/// 对上层而言无区别」。**那个判断在 F2 被推翻了**：AA4C 唯一不可替代的能力就是
/// 「我的几台设备在任何网络下自己连成一片」，而这条能力在界面上完全看不见——设备卡片
/// 只显示在线/离线，用户没法知道自己现在是在局域网里、还是真的跨网连上了、
/// 又或者只是在绕中继（慢，而且要自建服务器）。合并掉的正是最该让人看见的那一档。
///
/// 分档对应 CONNECT_DESIGN.md §2 的连接阶梯，从好到差：
/// 1. [`Lan`](Self::Lan) —— 同一个局域网，最快，不出网。
/// 2. [`PublicV4`](Self::PublicV4) / [`PublicV6`](Self::PublicV6) —— 对端有可直达的公网
///    地址。IPv4 与 IPv6 分开是因为国内家宽普遍下发公网 IPv6 而 IPv4 在 CGNAT 后面
///    （V0.7 R1 打通双栈的全部理由），「走的是 IPv6」对用户是有意义的信息，
///    排查连不上时也是最有用的一条。
/// 3. [`Punch`](Self::Punch) —— 打洞打出来的直连。最终也是直连，但经历了候选交换。
/// 4. [`Relay`](Self::Relay) —— 经自建服务器中继兜底，最慢。
///
/// **扁平枚举，不是带字段的变体**：这个值在 JSON 里是一个字符串（`"lan"` /
/// `"public_v4"` / …），前端按字符串字面量联合消费——那是本项目既有的约定
/// （见下方 `transfer_connected_json_shape` 测试）。把 IPv4/IPv6 做成变体里的
/// 一个 `bool` 会让它变成对象，既破坏约定，渲染时也还得再拆一层。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionVia {
    /// 局域网直连。
    Lan,
    /// 公网直连，走 IPv4。
    PublicV4,
    /// 公网直连，走 IPv6。
    PublicV6,
    /// NAT 打洞后升级成的 QUIC 直连（里程碑 C5）。
    Punch,
    /// 经自建服务器中继（里程碑 C3）。
    Relay,
}

impl ConnectionVia {
    /// 由一个已经连上的对端地址判定属于哪一档直连。
    ///
    /// 判据就是地址本身：回环、私有网段、链路本地、IPv6 ULA 都算局域网，其余算公网。
    /// 不去问「这个地址是从 mDNS 来的还是从服务器查来的」——来源能提示但不能决定：
    /// 服务器上注册的完全可能是一个私有地址（两台设备恰好同网段），mDNS 理论上也能
    /// 播一个公网地址。**看地址比看来源准。**
    pub fn direct_from_addr(addr: &std::net::SocketAddr) -> Self {
        use std::net::IpAddr;
        match addr.ip() {
            IpAddr::V4(v4) => {
                if v4.is_private() || v4.is_loopback() || v4.is_link_local() {
                    Self::Lan
                } else {
                    Self::PublicV4
                }
            }
            IpAddr::V6(v6) => {
                let lan = v6.is_loopback()
                    // fe80::/10 链路本地
                    || (v6.segments()[0] & 0xffc0) == 0xfe80
                    // fc00::/7 唯一本地地址（ULA）
                    || (v6.segments()[0] & 0xfe00) == 0xfc00;
                if lan {
                    Self::Lan
                } else {
                    Self::PublicV6
                }
            }
        }
    }

    /// 是不是直连（相对「绕中继」而言）。中继要自建服务器、而且慢，
    /// 界面上值得单独提示，所以这个判断会被反复用到。
    pub fn is_direct(&self) -> bool {
        !matches!(self, Self::Relay)
    }
}

/// AI 引擎的两个独立槽位（对话/嵌入，ARCHIVE_DESIGN.md §3.3）——各自独立
/// 进程、独立模型、独立生命周期，一个槽位的状态变化不影响另一个。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiSlot {
    Chat,
    Embedding,
}

/// AI 引擎槽位的生命周期状态（ARCHIVE_DESIGN.md §3.3：懒启动 + 空闲自停）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiEngineStatus {
    /// 正在拉起进程 / 等待模型加载完成健康检查通过。
    Starting,
    /// 健康检查通过，可以接受推理请求。
    Ready,
    /// 空闲超时后已优雅退出（正常降级，不是错误）。
    Stopped,
    /// 模型未配置 / 启动失败 / 健康检查超时——同下载能力缺失的既有
    /// `Aa4cError::Unavailable` 语义。
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum CoreEvent {
    DeviceFound(DeviceInfo),
    #[serde(rename_all = "camelCase")]
    DeviceLost {
        id: DeviceId,
    },
    DeviceUpdated(DeviceInfo),

    /// 对方请求与本机配对。
    #[serde(rename_all = "camelCase")]
    PairingRequest {
        session_id: String,
        peer: DeviceInfo,
    },
    /// 双方界面需要展示的 6 位确认码。
    #[serde(rename_all = "camelCase")]
    PairingPin {
        session_id: String,
        pin: String,
    },
    #[serde(rename_all = "camelCase")]
    PairingResult {
        session_id: String,
        peer: DeviceId,
        success: bool,
    },

    /// 对方请求向本机发送文件。
    #[serde(rename_all = "camelCase")]
    TransferRequest {
        task: TransferTask,
    },
    /// 出站连接已建立、即将开始传输（里程碑 C4 连接质量，CONNECT_DESIGN.md §2/§12）：
    /// 只在**发起方**（send/fetch）触发一次，告诉 UI 这次实际走的是哪一档，仅存于当次
    /// 会话内存（不落库，历史记录不含这个字段——见 HANDOFF.md 的取舍）。
    #[serde(rename_all = "camelCase")]
    TransferConnected {
        task_id: TaskId,
        via: ConnectionVia,
    },
    #[serde(rename_all = "camelCase")]
    TransferProgress {
        task_id: TaskId,
        transferred_bytes: u64,
        total_bytes: u64,
        speed_bps: u64,
        current_file: String,
    },
    #[serde(rename_all = "camelCase")]
    TransferDone {
        task_id: TaskId,
    },
    #[serde(rename_all = "camelCase")]
    TransferFailed {
        task_id: TaskId,
        error: String,
    },
    /// 发送方主动暂停——区别于 `TransferFailed`：不是错误，接收端的 `.aa4c-part`
    /// 已保留，随时可以「继续」接着传（见 `TransferStatus::Paused`）。
    #[serde(rename_all = "camelCase")]
    TransferPaused {
        task_id: TaskId,
    },

    /// 本机同步索引发生变化（扫描完成），UI 应重新拉取统一文件视图。
    SyncIndexUpdated,

    /// 某台设备的可达性变了（V0.8「Focus」F2）：能不能连上、走的哪一档。
    /// **只在确实变了时才发**——探测是每 30 秒一轮的，每轮都发会让首页的状态图
    /// 无谓地闪（同 `IntroductionsUpdated` 的既有取舍）。UI 收到后重新拉快照。
    ReachabilityUpdated,

    /// 收到了新的设备引荐（TRUST_DESIGN.md §5，里程碑 R2），UI 应重新拉取待确认列表。
    /// 只在**确实新增**了待确认记录时发——周期交换每轮都会把同一批引荐再收一遍，
    /// 每轮都发事件会让界面无谓地闪。
    IntroductionsUpdated,

    /// 下载进度（V0.4 里程碑 D1，DOWNLOAD_DESIGN.md §5）：状态迁移必发，进行中按数秒级
    /// 节流（不落库，前端本地维护——同 `TransferProgress` 的既有先例）。
    /// `seeders`/`peers`/`ratio` 是 D2（Transmission/BT）专属字段，HTTP 任务恒为
    /// `None` 且不出现在 JSON 里（`skip_serializing_if`）——同 `save_path` 不落库、
    /// 只进事件的既有先例（DOWNLOAD_DESIGN.md §3.6.4：BT 专属信息只进事件不落库），
    /// 复用同一个事件变体而不是另开一个 `BtProgress`，让 D1+D2 任务在前端走同一条
    /// 进度处理路径（对应 D3「统一任务中心」的目标）。
    #[serde(rename_all = "camelCase")]
    DownloadProgress {
        task_id: TaskId,
        downloaded_bytes: u64,
        total_bytes: u64,
        speed_bps: u64,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        seeders: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        peers: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        ratio: Option<f64>,
    },
    #[serde(rename_all = "camelCase")]
    DownloadDone {
        task_id: TaskId,
        save_path: String,
    },
    #[serde(rename_all = "camelCase")]
    DownloadFailed {
        task_id: TaskId,
        error: String,
    },

    /// 一次归档移动生效（V0.5 里程碑 AI1，ARCHIVE_DESIGN.md §2.4）：自动（下载完成钩子）
    /// 或手动归档都会发这条，UI 据此刷新归档列表；`rule_id` 为 `None` 代表手动归档。
    #[serde(rename_all = "camelCase")]
    ArchiveApplied {
        entry_id: String,
        from_path: String,
        to_path: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        rule_id: Option<String>,
    },

    /// AI 引擎槽位状态变化（V0.5 里程碑 AI2，ARCHIVE_DESIGN.md §3.3）：懒启动/
    /// 空闲自停都经这条通知 UI（"正在加载模型…"/引导去模型库）。
    #[serde(rename_all = "camelCase")]
    AiEngineState {
        slot: AiSlot,
        status: AiEngineStatus,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        error: Option<String>,
    },

    /// AI 标签/分类建议批量队列进度（V0.5 里程碑 AI3，ARCHIVE_DESIGN.md §5）：
    /// 单并发逐个调用，每处理完一个文件发一次，`done == total` 即批量结束
    /// （不单独发"批量完成"事件，前端自己比较两个数字）。
    #[serde(rename_all = "camelCase")]
    AiSuggestProgress {
        done: u32,
        total: u32,
    },

    /// 知识库摄入进度（V0.5 里程碑 AI4，ARCHIVE_DESIGN.md §6）：单个来源目录内
    /// 逐文档嵌入，每处理完一个文档发一次，语义同 `AiSuggestProgress`
    /// （`done == total` 即这个来源摄入完成，不单独发"完成"事件）。
    #[serde(rename_all = "camelCase")]
    KbIngestProgress {
        source_id: String,
        done: u32,
        total: u32,
    },

    /// 知识库问答流式增量（V0.5 里程碑 AI4，ARCHIVE_DESIGN.md §6）：对话槽位
    /// SSE 转发，`request_id` 供前端关联到发起的那次提问（同一时刻只支持一个
    /// 进行中的问答，`request_id` 仍然带上是为了让前端能安全丢弃过期请求的
    /// 迟到增量，不是为了支持真正的并发问答）。
    #[serde(rename_all = "camelCase")]
    KbAnswerDelta {
        request_id: String,
        delta: String,
    },

    /// 知识库问答结束：附带引用来源列表（去重后的文件路径）。引擎失败/超时
    /// 时 `error` 非空，`sources` 为空数组。
    #[serde(rename_all = "camelCase")]
    KbAnswerDone {
        request_id: String,
        sources: Vec<KbAnswerSource>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        error: Option<String>,
    },
}

impl CoreEvent {
    /// Tauri 事件名后缀（API_DESIGN.md §9.2：`aa4c://` + 蛇形事件名）。
    pub fn event_name(&self) -> &'static str {
        match self {
            Self::DeviceFound(_) => "device_found",
            Self::DeviceLost { .. } => "device_lost",
            Self::DeviceUpdated(_) => "device_updated",
            Self::PairingRequest { .. } => "pairing_request",
            Self::PairingPin { .. } => "pairing_pin",
            Self::PairingResult { .. } => "pairing_result",
            Self::TransferRequest { .. } => "transfer_request",
            Self::TransferConnected { .. } => "transfer_connected",
            Self::TransferProgress { .. } => "transfer_progress",
            Self::TransferDone { .. } => "transfer_done",
            Self::TransferFailed { .. } => "transfer_failed",
            Self::TransferPaused { .. } => "transfer_paused",
            Self::SyncIndexUpdated => "sync_index_updated",
            Self::IntroductionsUpdated => "introductions_updated",
            Self::ReachabilityUpdated => "reachability_updated",
            Self::DownloadProgress { .. } => "download_progress",
            Self::DownloadDone { .. } => "download_done",
            Self::DownloadFailed { .. } => "download_failed",
            Self::ArchiveApplied { .. } => "archive_applied",
            Self::AiEngineState { .. } => "ai_engine_state",
            Self::AiSuggestProgress { .. } => "ai_suggest_progress",
            Self::KbIngestProgress { .. } => "kb_ingest_progress",
            Self::KbAnswerDelta { .. } => "kb_answer_delta",
            Self::KbAnswerDone { .. } => "kb_answer_done",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_json_shape_matches_api_design() {
        let event = CoreEvent::TransferProgress {
            task_id: "t1".into(),
            transferred_bytes: 500,
            total_bytes: 1000,
            speed_bps: 42_000_000,
            current_file: "IMG_2024.jpg".into(),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "transfer_progress");
        assert_eq!(json["data"]["taskId"], "t1");
        assert_eq!(json["data"]["transferredBytes"], 500);
        assert_eq!(json["data"]["speedBps"], 42_000_000);
        assert_eq!(event.event_name(), "transfer_progress");

        let back: CoreEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, event);
    }

    #[test]
    fn event_name_matches_serde_tag() {
        let event = CoreEvent::DeviceLost { id: "d1".into() };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], event.event_name());
    }

    /// 连接档位由**地址本身**判定，不看它是从哪查来的（V0.8 F2）。
    ///
    /// 私有网段 / 回环 / 链路本地 / IPv6 ULA 都算局域网，其余算公网——这条直接决定
    /// 首页状态图上每台设备显示「局域网」还是「公网」，判错就是在骗用户。
    #[test]
    fn the_rung_is_decided_by_the_address_not_by_where_it_came_from() {
        use std::net::SocketAddr;
        let via = |s: &str| ConnectionVia::direct_from_addr(&s.parse::<SocketAddr>().unwrap());

        // 局域网：三段私有 IPv4 + 回环 + 链路本地
        assert_eq!(via("192.168.1.5:42420"), ConnectionVia::Lan);
        assert_eq!(via("10.0.0.7:42420"), ConnectionVia::Lan);
        assert_eq!(via("172.16.3.9:42420"), ConnectionVia::Lan);
        assert_eq!(via("127.0.0.1:42420"), ConnectionVia::Lan);
        assert_eq!(via("169.254.1.1:42420"), ConnectionVia::Lan);
        // 172.32 不在 172.16/12 里——这是最容易写错的一条边界
        assert_eq!(via("172.32.0.1:42420"), ConnectionVia::PublicV4);

        // 局域网：IPv6 回环 / 链路本地 fe80::/10 / ULA fc00::/7
        assert_eq!(via("[::1]:42420"), ConnectionVia::Lan);
        assert_eq!(via("[fe80::1]:42420"), ConnectionVia::Lan);
        assert_eq!(via("[fd12:3456::1]:42420"), ConnectionVia::Lan);

        // 公网
        assert_eq!(via("203.0.113.7:42420"), ConnectionVia::PublicV4);
        assert_eq!(via("[2001:db8::1]:42420"), ConnectionVia::PublicV6);
    }

    /// 中继是唯一的非直连档——界面上要单独提示（慢，而且要自建服务器）。
    #[test]
    fn only_relay_is_not_direct() {
        assert!(ConnectionVia::Lan.is_direct());
        assert!(ConnectionVia::PublicV4.is_direct());
        assert!(ConnectionVia::PublicV6.is_direct());
        assert!(ConnectionVia::Punch.is_direct());
        assert!(!ConnectionVia::Relay.is_direct());
    }

    /// 里程碑 C4：连接质量事件的 JSON 形状（camelCase `taskId` + snake_case 的 via 取值），
    /// 前端 `ConnectionVia` 类型按这个约定做字符串字面量联合。
    #[test]
    fn transfer_connected_json_shape() {
        let event = CoreEvent::TransferConnected {
            task_id: "t1".into(),
            via: ConnectionVia::Relay,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "transfer_connected");
        assert_eq!(json["data"]["taskId"], "t1");
        assert_eq!(json["data"]["via"], "relay");
        // 细化出来的几档同样是**字符串**，不是对象——前端的字面量联合依赖这一点。
        for (v, want) in [
            (ConnectionVia::Lan, "lan"),
            (ConnectionVia::PublicV4, "public_v4"),
            (ConnectionVia::PublicV6, "public_v6"),
            (ConnectionVia::Punch, "punch"),
        ] {
            assert_eq!(serde_json::to_value(v).unwrap(), serde_json::json!(want));
        }
        assert_eq!(event.event_name(), "transfer_connected");

        let back: CoreEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, event);
    }

    /// 里程碑 D1：下载进度事件的 JSON 形状（camelCase，同 `TransferProgress` 的既有约定）。
    /// HTTP 任务（D1）没有做种数/peer 数/分享率，三个字段应该整体不出现在 JSON 里
    /// （不是出现成 `null`）——`skip_serializing_if` 就是为了这一点。
    #[test]
    fn download_progress_json_shape() {
        let event = CoreEvent::DownloadProgress {
            task_id: "gid1".into(),
            downloaded_bytes: 500,
            total_bytes: 1000,
            speed_bps: 1_000_000,
            seeders: None,
            peers: None,
            ratio: None,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "download_progress");
        assert_eq!(json["data"]["taskId"], "gid1");
        assert_eq!(json["data"]["downloadedBytes"], 500);
        assert!(json["data"].get("seeders").is_none());
        assert!(json["data"].get("peers").is_none());
        assert!(json["data"].get("ratio").is_none());
        assert_eq!(event.event_name(), "download_progress");

        let back: CoreEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, event);
    }

    /// D2：BT 任务的做种数/peer 数/分享率只进事件、不落库（DOWNLOAD_DESIGN.md
    /// §3.6.4），复用同一个 `DownloadProgress` 变体而不是另开一个 BT 专属事件。
    #[test]
    fn download_progress_json_shape_with_bt_fields() {
        let event = CoreEvent::DownloadProgress {
            task_id: "infohash1".into(),
            downloaded_bytes: 500,
            total_bytes: 1000,
            speed_bps: 1_000_000,
            seeders: Some(12),
            peers: Some(3),
            ratio: Some(1.5),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["data"]["seeders"], 12);
        assert_eq!(json["data"]["peers"], 3);
        assert_eq!(json["data"]["ratio"], 1.5);

        let back: CoreEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, event);
    }

    /// 里程碑 AI2：AI 引擎槽位状态事件的 JSON 形状，`error` 缺省时整体不出现
    /// 在 JSON 里（同 `rule_id`/BT 专属字段的既有先例）。
    #[test]
    fn ai_engine_state_json_shape() {
        let event = CoreEvent::AiEngineState {
            slot: AiSlot::Embedding,
            status: AiEngineStatus::Starting,
            error: None,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "ai_engine_state");
        assert_eq!(json["data"]["slot"], "embedding");
        assert_eq!(json["data"]["status"], "starting");
        assert!(json["data"].get("error").is_none());
        assert_eq!(event.event_name(), "ai_engine_state");

        let back: CoreEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, event);
    }

    /// 里程碑 AI3：批量建议进度事件的 JSON 形状——两个裸数字，没有可省略字段。
    #[test]
    fn ai_suggest_progress_json_shape() {
        let event = CoreEvent::AiSuggestProgress { done: 2, total: 5 };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "ai_suggest_progress");
        assert_eq!(json["data"]["done"], 2);
        assert_eq!(json["data"]["total"], 5);
        assert_eq!(event.event_name(), "ai_suggest_progress");

        let back: CoreEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, event);
    }

    /// 里程碑 AI4：知识库摄入进度事件的 JSON 形状。
    #[test]
    fn kb_ingest_progress_json_shape() {
        let event = CoreEvent::KbIngestProgress {
            source_id: "src-1".into(),
            done: 3,
            total: 10,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "kb_ingest_progress");
        assert_eq!(json["data"]["sourceId"], "src-1");
        assert_eq!(json["data"]["done"], 3);
        assert_eq!(json["data"]["total"], 10);
        assert_eq!(event.event_name(), "kb_ingest_progress");

        let back: CoreEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, event);
    }

    /// 里程碑 AI4：问答完成事件——`error` 缺省不出现在 JSON 里，同其余
    /// `skip_serializing_if` 字段的既有约定。
    #[test]
    fn kb_answer_done_json_shape_omits_absent_error() {
        let event = CoreEvent::KbAnswerDone {
            request_id: "req-1".into(),
            sources: vec![KbAnswerSource {
                path: "/tmp/notes/a.md".into(),
            }],
            error: None,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "kb_answer_done");
        assert_eq!(json["data"]["requestId"], "req-1");
        assert_eq!(json["data"]["sources"][0]["path"], "/tmp/notes/a.md");
        assert!(json["data"].get("error").is_none());
        assert_eq!(event.event_name(), "kb_answer_done");

        let back: CoreEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, event);
    }
}

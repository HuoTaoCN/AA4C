# AA4C 归档与 AI 设计（V0.5「AI」）

> 状态：**里程碑 AI1（规则式归档，无 AI）已实现**；AI2–AI5（AI 引擎/建议/知识库/发布）仍是设计稿。对应 [ROADMAP.md](ROADMAP.md) V0.5（AI 归档：自动分类 / 标签 / 模型管理 / 本地知识库）；实现拆解见 [V0.5_IMPLEMENTATION_PLAN.md](V0.5_IMPLEMENTATION_PLAN.md)（里程碑 AI1–AI5）。
>
> **本文档的关键外部事实已在规划阶段真机实证（不是从网页/文档抄的），标注在 §3.1**：llama.cpp 官方 release 对我们全部目标平台提供预编译二进制（这一点直接决定了引擎分发方案——对照 V0.4 D1 的教训：aria2 官方"有二进制"的说法对 2/3 平台不成立，被迫自建整条构建流水线）；`llama-server` 的环境变量配置与 `LLAMA_API_KEY` 已在本机真实二进制上验证。实现期仍需补验证的项集中列在 §11。AI1 落地时又补了一处实证：`general.file_type` 的量化枚举/名称表直接抓取 llama.cpp `master` 分支的 `include/llama.h`（`enum llama_ftype`）与 `src/llama-model-loader.cpp`（`llama_ftype_name()`），不是凭记忆猜的，见 §2.2 与 `crates/aa4c-core/src/archive/gguf.rs` 模块文档。
>
> **AI1 实现偏差**（相对本文档 v1 定稿的小出入，均为实现期发现的必要补充，不是推翻设计）：① `list_archive_log` 补成第 7 个 Command——§9/V0.5_IMPLEMENTATION_PLAN 原列的 6 个 Command 里漏了这条，但归档页"最近动作"要展示每条历史并挂"撤销"按钮，前端必须能拿到 `log_id`，`aa4c-store::list_archive_log` 早就有、只是没接 Tauri 层；② 新增 `apply_selected_rule`（引擎内部函数，不是新 Command）——手动归档的"应用某条规则"路径需要**跳过**该规则自己的匹配条件强制执行（用户主动选了就是要覆盖自动匹配结果），复用 `apply_rules`/`apply_manual` 共用的落库尾段（`record_move`/`finish_move`），细节见引擎源码注释；③ 公历年/月不引入 `chrono`，手写 Howard Hinnant 的 `civil_from_days` 算法（已用 5 个真实日期交叉验证，含世纪闰年边界与纪元本身），维持"不为小需求加依赖"的一贯克制。
>
> 给执行 Agent：本文 §10 是已确认决策，**不要重开已定案的讨论**；发现实现与设计冲突时，参照 DOWNLOAD_DESIGN.md v1→v2→v3 的先例——先把冲突事实核实清楚，把修订写进本文档（记入"实现偏差"），再改代码。

## 0. 定位与边界

**归档是连接之上的第五种能力**（传输 / 同步 / 分享 / 下载 / **归档**）。它回答的问题是：文件进来之后（下载完成、传输收到、手动指定），**放哪、叫什么、怎么找回来**。

- **规则自动、AI 建议**——这是全篇最重要的一条原则：确定性的规则引擎（按类别/扩展名/文件名模式匹配 → 移动/打标签）可以自动执行；AI 的产出（分类建议、标签建议、知识库回答）**永远只是建议**，落到"待确认"队列由用户点头后才生效。AI 不会在无人确认的情况下移动、改名、删除任何文件。
- **完全本地**——llama.cpp 跑本地 GGUF 模型，零云端调用、零遥测。不接任何在线 AI API（OpenAI/Claude/千问在线版都不接）；这是产品"用户数据属于用户自己"边界的自然延伸，不是成本考量。
- **不是**：内容平台、模型市场、云端知识库。模型文件由用户自己经下载中心获取（我们只推荐 + 识别 + 管理，不分发）。
- 与 **Lua 插件系统（DOWNLOAD_DESIGN.md §10，暂记 D4）的关系**：插件是站点化长尾（PT 站、站内搜索）的载体，归档是内建能力，两者独立；插件的 `on_task_complete` 钩子未来可以调用归档动作，但 V0.5 不实现插件，不为它预留超出 §10 已定边界的额外接缝。
- **平台范围：桌面三平台（Windows / macOS / Linux）**，同 V0.4 先例。Android 不含（llama.cpp 官方有 android-arm64 产物，但 Android 打包路径没有桌面 sidecar 机制，需单独评估，见 §11）。`aa4c-server`/Docker 的 headless AI 同样后置（`ProcessSpawner` 路径天然支持，不需要现在做）。

## 1. 分层与 crate 归属

照抄既有能力的归属逻辑（同步=纯逻辑住 `aa4c-core`，下载=外部进程住独立 crate）：

| 部分 | 归属 | 理由 |
|------|------|------|
| 归档规则引擎（AI1，无 AI） | `aa4c-core::archive` 模块 + `aa4c-store` 新表 | 纯业务逻辑 + fs 操作，同 `sync_index`/`unified` 先例，不值得开 crate |
| 文件类型识别 + GGUF 解析（AI1） | `aa4c-types` 或 `aa4c-core::archive` 内部模块 | 无依赖纯函数；放 `aa4c-core::archive::detect` / `archive::gguf`，除非别的 crate 要用再上提 |
| sidecar 公共设施（AI2 重构） | **新 crate `aa4c-engine`**：`SidecarSpawner`/`EngineChild`/`ProcessSpawner`/`orphan_guard` 从 `aa4c-download` 平移过来 | `aa4c-ai` 也要拉外部进程，不能让 ai 依赖 download；纯机械重构，行为零变化，全部既有测试原样通过是重构完成的判据 |
| AI 引擎（AI2+） | **新 crate `aa4c-ai`**（不依赖 Tauri，镜像 `aa4c-download` 的形态） | 外部进程（llama-server）+ RPC 客户端 + 生命周期管理 |
| 知识库（AI4） | `aa4c-ai::kb` | 强依赖嵌入引擎 |
| 桌面壳层 | `TauriSidecarSpawner` 已按名字参数化（D2 起），第三个 sidecar 直接复用 | — |

依赖方向：`aa4c-types` ← `aa4c-engine` ← (`aa4c-download`, `aa4c-ai`) ← `aa4c-core` ← 壳层。无环。

## 2. 规则式归档引擎（里程碑 AI1，无 AI 也完整可用）

### 2.1 类别体系（内置，不可增删——标签才是用户的自由维度）

`模型 / 图片 / 视频 / 音频 / 文档 / 电子书 / 压缩包 / 安装包 / 代码 / 字幕 / 其他`。

识别 = 扩展名表（主）+ 少量 magic bytes 兜底（手写 ~20 条，不引 `infer` 等新依赖——项目一贯克制）：GGUF magic（`0x46554747` LE，见 §2.2）、safetensors（8 字节头长 + `{`）、zip/rar/7z/gzip、png/jpg/webp、mp4/mkv、pdf 等。扩展名与 magic 冲突时信 magic。

### 2.2 GGUF 元数据解析（「模型管理」的根基，纯 Rust 手写，无新依赖）

GGUF 头格式（只读头部，**永远不读张量数据**，几 GB 的文件只读前几十 KB）：

```
magic:  u32  = 0x46554747 ("GGUF" LE)
version: u32 （只支持 2 和 3；v1 的长度字段是 u32 不兼容，直接报"版本过旧"）
tensor_count: u64
metadata_kv_count: u64
随后 metadata_kv_count 个 KV：
  key:   string（u64 长度 + UTF-8 字节）
  type:  u32（0..=12：U8,I8,U16,I16,U32,I32,F32,BOOL,STRING,ARRAY,U64,I64,F64）
  value: 按 type；ARRAY = u32 元素类型 + u64 个数 + 逐个元素
```

- 关心的 key（其余跳过）：`general.architecture`、`general.name`、`general.size_label`（如 "4B"，常见于 HF 产物但非必有）、`general.file_type`（量化枚举，映射常见值 Q4_K_M/Q8_0 等；**枚举值以 llama.cpp 源码常量为准，实现期核对**，未知值显示原始数字并从文件名回退解析量化标签）、`<arch>.context_length`。
- **防御性硬界限**（模型文件可能来路不明）：kv_count ≤ 4096、单字符串 ≤ 64 KiB、数组元素 ≤ 65536，越界即报"文件头异常"而不是继续分配内存；解析全程 `BufReader` 顺序读，失败不 panic。
- 单元测试用手工构造的最小 GGUF 头字节（不需要真模型文件）+ 越界样本。

### 2.3 规则模型

规则 = **匹配条件**（类别 ∈ 集合；可选扩展名集合、文件名 glob、大小上下限）+ **动作**（移动到目标目录模板 + 追加标签列表）。按 `position` 顺序取**第一条**命中的规则执行（不叠加，简单可预测）。

目标目录模板支持占位符：`{类别}`、`{年}`、`{月}`、`{扩展名}`；模型类别额外有 `{模型.架构}`（qwen3/llama…）、`{模型.名称}`、`{模型.量化}`——旗舰场景（PROJECT_VISION §5）：`Qwen3-4B-Instruct-Q4_K_M.gguf` 下载完成 → 命中"模型"规则 → 移入 `<归档根>/模型/qwen3/`，模型库立即可见。占位符取不到值时用 `未知`，绝不失败中断。

**内置预设规则（模型/图片/视频/文档/压缩包五条）随首次启动写入但全部默认停用**；归档页空态给「一键启用推荐规则」。理由：装完就悄悄移动用户文件是意外行为，违背项目"隐私优先/不意外"的一贯温度；一键启用让旗舰场景只差一次点击。

### 2.4 触发时机与移动语义

- **自动触发只有一个入口：下载完成**。`aa4c-core` 内部起一个后台任务订阅自己的事件总线（同 `sync_exchange` 后台循环先例），收到 `DownloadDone` → 跑规则引擎。**不给同步/传输收件自动挂规则**——收到的文件在 Inbox 索引根内，移走 = 同步侧看见删除并向其它设备传播，属于"无人值守的数据意外"；手动归档不受此限（见下）。
- **手动归档**：归档页/统一文件视图选中任意文件 → 应用某条规则或手选目标。文件在同步范围内时弹**不阻断的警示**（"移动后其它设备会看到此文件被删除"——照抄 D3 下载目录警示的"警示不硬禁"原则与实现手法：前端用既有 `list_sync_scopes` 做路径前缀比对）。
- **移动 = `std::fs::rename`，跨卷（EXDEV）回退 copy+fsync+delete**；目标已存在同名文件时加序号（`报告 (2).pdf`，复用 `unified.rs` 的既有命名逻辑）。移动成功后：更新 `download_tasks.save_path`（`aa4c-store` 新方法；不更新的话「打开所在文件夹」就指向空位），写一条 `archive_log`（含原路径/新路径/规则 id），发 `CoreEvent::ArchiveApplied`。
- **撤销**：`archive_log` 按条撤销 = 反向移动 + 回写 save_path + 摘掉本次追加的标签。原位置已被占用则报错不强行覆盖。

### 2.5 归档根目录

`Settings` 新增 `archive_root: Option<String>`，默认 `<系统文档目录>/AA4C归档`（`dirs::document_dir()`，同 `download_dir` 用系统下载目录的直觉）。**必须与 `save_dir` 子树、`download_dir` 互不嵌套**——落进 Inbox 索引根 = 自动分享给完全信任设备（同 DOWNLOAD_DESIGN §5 的既有分析）；设置页换目录时做同款警示。

## 3. AI 引擎（llama.cpp / llama-server，里程碑 AI2）

### 3.1 已实证的关键事实（2026-07-21，规划阶段真机验证）

1. **llama.cpp 官方 release（tag `b10069`）对全部目标平台提供预编译产物**：`llama-<tag>-bin-macos-arm64.tar.gz`、`-macos-x64.tar.gz`、`-ubuntu-x64.tar.gz`（另有 arm64/vulkan 变体）、`-win-cpu-x64.zip`（另有 CUDA/Vulkan 变体）、甚至 `-android-arm64.tar.gz`。**不需要像 aria2/Transmission 那样自建源码构建流水线**，engines.yml 只需"下载官方产物 → 裁剪 → 重新发布到我们自己的 engines release + 校验和"这一条轻量腿。
2. **产物形态 = 可执行文件 + 一批动态库**（macos-arm64 包实测：`llama-server` + `libllama*.dylib`/`libggml*.dylib`（含 `libggml-metal`）等约 30 个文件 + 一堆我们不需要的 `llama-cli`/`llama-bench` 等工具）。`llama-server` 的 rpath 实测为 `@loader_path`（库在可执行文件旁边即可加载）——**与 Transmission 完全同形**，D2.8 的全部先例直接适用：`externalBin`（裁剪出的 `llama-server`）+ `bundle.resources`（`-libs/` 目录）+ `TauriSidecarSpawner` 运行时注入 `DYLD_LIBRARY_PATH`/`LD_LIBRARY_PATH`。
3. **`llama-server` 全套参数支持环境变量配置**（本机运行 `--help` 实测）：`LLAMA_ARG_MODEL`/`LLAMA_ARG_PORT`/`LLAMA_ARG_HOST`/`LLAMA_ARG_EMBEDDINGS`/`LLAMA_ARG_CTX_SIZE`/`LLAMA_ARG_THREADS`/…以及 **`LLAMA_API_KEY`**。密钥经环境变量传递，**不走命令行参数**（命令行对本机任意进程经 `ps` 可见——DOWNLOAD_DESIGN v2 §7 的 rpc-secret 教训，这里靠环境变量满足，连配置文件都不用写）。
4. llama.cpp 许可证 **MIT**——比 GPL 引擎更宽松，sidecar 进程隔离照旧（架构一致性，不是许可证被迫）。

### 3.2 进程与 RPC

- 启动：`llama-server` 由 `TauriSidecarSpawner`（桌面）/`ProcessSpawner`（测试/headless）拉起，配置全走环境变量：`LLAMA_ARG_HOST=127.0.0.1`、随机端口、随机 `LLAMA_API_KEY`（uuid 拼 32 字节 base58，同分享 token 生成法）、`LLAMA_ARG_MODEL=<模型路径>`、`LLAMA_ARG_CTX_SIZE=8192`；嵌入槽位额外 `LLAMA_ARG_EMBEDDINGS=1`。
- 孤儿防护：复用 `aa4c-engine::orphan_guard` 三平台路径（Windows Job Object / Linux `PR_SET_PDEATHSIG` / macOS PID 文件），llama-server 与 transmission-daemon 一样没有 `stop-with-process` 类机制。
- RPC：**手写极简 HTTP/1.1 客户端**（照抄 `TransmissionClient` 先例，不引 reqwest）。用的端点：`GET /health`（就绪门），`POST /v1/chat/completions`（OpenAI 形态；批量任务非流式、知识库问答走 SSE 流式——`data: {...}` 行解析，`[DONE]` 结尾），`POST /v1/embeddings`。鉴权 `Authorization: Bearer <key>`。
- **结构化输出**：分类/标签建议请求带 JSON Schema 约束（llama-server 支持 grammar 约束采样，输出保证合法 JSON，杜绝"解析模型自由发挥"这类脆弱代码）。具体请求字段名（`response_format` vs `json_schema`）实现期以 `--help`/实测为准（§11 待验证项）。
- 就绪时间：模型加载可能要几十秒（CPU + 数 GB 模型），健康检查轮询上限设 120s；推理请求超时 300s（CPU 慢是常态，不是错误）。

### 3.3 生命周期：懒启动 + 空闲自停（与下载引擎的关键差异）

下载引擎轻量常驻；**LLM 引擎吃内存（4B Q4 模型 ≈ 3 GB RAM），绝不常驻**。`AiService` 管两个**槽位**（对话/嵌入，各自独立进程、独立模型）：

- 首个 AI 请求到来 → 拉起对应槽位（`AiEngineState` 事件：`starting → ready`，前端显示"正在加载模型…"）；
- 空闲超过 `ai_idle_timeout_minutes`（默认 10）→ 优雅退出进程释放内存；
- 模型未配置/加载失败 → `Unavailable` 语义（同下载能力缺失的既有错误处理路径），UI 引导去模型库。

### 3.4 打包分发（engines.yml 轻量腿）

- `engines.yml` 新增 llama 腿：**下载官方 release 产物 →（macOS 用官方 arm64+x64 两包逐文件 `lipo` 出 universal，做法对照 `fetch-engines.sh` 头部注释里 Tauri 不替 externalBin 自动 lipo 的既有说明）→ 裁剪到只剩 `llama-server` + 它实际依赖的库（`otool -L`/`ldd` 逐项核对，不整包塞进去）→ 重新打 zip + SHA256SUMS → 发布到我们自己的 `engines/llama-<tag>` release**。锁定一个具体 tag（llama.cpp 每天发版，我们按里程碑手动升级），校验和写死进 `fetch-engines.sh`（既有惯例）。
- 变体选择：**全平台 CPU 基线**（macOS 官方 arm64 包自带 Metal，白赚 GPU；Windows/Linux 的 CUDA/Vulkan 变体后置，见 §11）。
- `tauri.conf.json`：第三个 `externalBin`（`llama-server`）+ `bundle.resources` 指向 `-libs/` 目录；`tauri.android.conf.json` 已整体清空 externalBin/resources，无需再动，但**改完必须跑一次 Android 哨兵确认**（D2.8 教训：resources glob 零匹配会让哨兵爆）。
- ⚠️ **Linux AppImage 预警**（新近教训的直接推论，见 HANDOFF.md 第五节 AppImage 一节）：linuxdeploy 打包阶段会扫描 `usr/bin/` 下每个可执行文件的 ELF 依赖并要求在**系统路径**可解析。transmission 的依赖是 apt 装得到的系统库，`release.yml` 装包就解决了；**`llama-server` 依赖的 `libllama/libggml*` 是我们自己的资源库，apt 装不到**——`release.yml` 的 ubuntu job 必须在 `tauri build` 前把 `-libs/` 目录加进 `LD_LIBRARY_PATH`（linuxdeploy 会尊重它）或采用等效手段，**并且必须用真实 CI 构建验证 AppImage 真的打出来了**，不要等发版才发现（这正是 preview.2 踩过的坑的变体）。
- CI（`ci.yml`）：externalBin 声明后任何触碰 `aa4c-desktop` 的 cargo 命令都要求二进制存在（D1 教训），三平台 job 照 transmission 先例取 llama-server：macOS `brew install llama.cpp`、Linux/Windows 下载官方 release 解包，经 `fetch-engines.sh --from-path` 就位。

### 3.5 模型管理（模型库）

- `Settings` 新增：`ai_models_dir`（默认 `<归档根>/模型`——与内置"模型"归档规则的目标目录**故意同址**：下载 GGUF → 自动归档进模型目录 → 模型库立即可见，一条龙）、`ai_chat_model` / `ai_embedding_model`（模型文件路径，null=未配置）。
- 模型库页：扫描 `ai_models_dir` 下 `.gguf`（递归一层），逐个读 GGUF 头展示（名称/架构/量化/上下文长度/文件大小）；选定对话模型/嵌入模型；显示引擎状态（未加载/加载中/就绪）。
- **推荐模型**（写死在前端的推荐卡片，给出可复制链接 + 一键"用下载中心下载"）：对话 `Qwen3-4B-Instruct` GGUF Q4_K_M（≈2.5GB，Apache-2.0）；嵌入 `Qwen3-Embedding-0.6B` GGUF Q8_0（≈0.6GB，Apache-2.0，中英双强）。**每个模型同时给 ModelScope（国内可达）与 Hugging Face 两个直链**；具体 URL 与文件名实现期核实（§11）。8GB 内存机器可跑 4B Q4；卡片上写清内存需求。

## 4. 数据模型（`aa4c-store`，两个迁移分属两个里程碑）

**迁移 009（AI1）**——归档：

```sql
CREATE TABLE archive_rules (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 0,
  position INTEGER NOT NULL,           -- 匹配顺序
  match_json TEXT NOT NULL,            -- {categories:[..], extensions:[..]?, glob:..?, min_size:..?, max_size:..?}
  action_json TEXT NOT NULL,           -- {target_template:.., tags:[..]}
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
CREATE TABLE archive_entries (         -- 一条被归档管理的文件记录
  id TEXT PRIMARY KEY, current_path TEXT NOT NULL, category TEXT NOT NULL,
  size INTEGER NOT NULL, model_meta_json TEXT,   -- GGUF 元数据，仅模型类别
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
CREATE TABLE archive_tags ( entry_id TEXT NOT NULL REFERENCES archive_entries(id) ON DELETE CASCADE,
  tag TEXT NOT NULL, source TEXT NOT NULL,       -- 'rule' | 'ai' | 'user'
  PRIMARY KEY (entry_id, tag) );
CREATE TABLE archive_log (             -- 移动历史，供撤销
  id INTEGER PRIMARY KEY AUTOINCREMENT, entry_id TEXT NOT NULL,
  from_path TEXT NOT NULL, to_path TEXT NOT NULL, rule_id TEXT,   -- NULL=手动
  at INTEGER NOT NULL, undone INTEGER NOT NULL DEFAULT 0 );
```

**迁移 010（AI4）**——知识库：

```sql
CREATE TABLE kb_sources ( id TEXT PRIMARY KEY, path TEXT NOT NULL, created_at INTEGER NOT NULL );
CREATE TABLE kb_documents ( id TEXT PRIMARY KEY, source_id TEXT NOT NULL, rel_path TEXT NOT NULL,
  mtime INTEGER NOT NULL, hash TEXT NOT NULL, status TEXT NOT NULL, updated_at INTEGER NOT NULL );
CREATE TABLE kb_chunks ( id INTEGER PRIMARY KEY AUTOINCREMENT, doc_id TEXT NOT NULL REFERENCES kb_documents(id) ON DELETE CASCADE,
  seq INTEGER NOT NULL, text TEXT NOT NULL, embedding BLOB NOT NULL, dims INTEGER NOT NULL );
```

AI 建议（AI3）**不落库**——待确认建议是易失的内存态（应用重启即清空，重新跑一次就有），避免为一个"队列"造表；用户确认后的产物（标签/移动）落上面的既有表。`Settings` 新 KV 键：`archive_root`、`archive_auto_enabled`（默认 true——真正的保守闸门在"预设规则默认停用"）、`ai_models_dir`、`ai_chat_model`、`ai_embedding_model`、`ai_idle_timeout_minutes`。

## 5. AI 标签 / 分类建议（里程碑 AI3）

- **输入构造**：文件名 + 类别 + 大小 +（文本族文件）开头 ≤8 KB 内容。**V0.5 无视觉**——图片/视频只按文件名与元数据建议（llama.cpp 多模态存在但范围失控，后置 §11）。
- **输出**：JSON Schema 约束的 `{category, tags: [..], reason}`；`temperature` 低（0.2）。
- **流程**：用户在归档页选文件（或"对最近下载的 N 个建议一下"）→ 进入批量队列（单并发，逐个调用，`CoreEvent::AiSuggestProgress`）→ 结果进"待确认"列表 → 用户逐条/批量 采纳（写标签、可选执行移动）或忽略。**采纳才落库/动文件**（§0 总原则）。
- 引擎不可用/超时：该文件标"建议失败"，不重试不阻塞队列（同 D3 批量操作"单个失败只跳过"的先例）。

## 6. 本地知识库（里程碑 AI4，最小可用版）

- **范围收紧到文本族**：md / txt / 代码 / json / csv 等 UTF-8 可读文件。**PDF/Office 不进 V0.5**（提取质量是无底洞，见 §11）。单文件读取上限 1 MB。
- **摄入**：用户添加来源目录（`kb_sources`，天然候选：归档根、同步范围）→ 扫描 → 按 mtime+hash 增量 → 分块（目标 ~1000 字符、重叠 ~200，段落边界优先）→ 嵌入槽位批量 `/v1/embeddings` → 存 `kb_chunks`（f32 LE BLOB）。进度事件 + 可随时中断（干净地停在文档边界）。
- **检索**：暴力余弦——SQLite 读全部 chunk 向量在 Rust 里算（个人规模 1 万 chunk × 1024 维 ≈ 40 MB 遍历，毫秒级；**5 万 chunk 起 UI 提示"知识库偏大"**）。不引向量库/SQLite 扩展——个人场景用不上，是本项目"简单>复杂"的直接应用。
- **问答**：query 嵌入 → top-6 chunk → 模板拼 prompt（"仅根据以下资料回答，资料不足就说不知道"）→ 对话槽位 SSE 流式 → `CoreEvent::KbAnswerDelta{request_id, delta}` / `KbAnswerDone{request_id, sources}`；UI 显示带来源文件路径的引用列表（点击打开所在文件夹）。**不做多轮对话记忆**（V0.5 每问独立）。
- ⚠️ **提示注入的姿态**：知识库内容是用户自己的文件，但仍可能含恶意文本（下载的 README 等）。系统性防御不现实也不必要——真正的安全边界是 §0 总原则：**LLM 输出只呈现给人看，永远不驱动任何自动动作**。此立场写进 SECURITY.md。

## 7. 安全与隐私

- 引擎仅绑 `127.0.0.1` + 随机端口 + 随机 `LLAMA_API_KEY`（环境变量传递，不经命令行/配置文件）。
- GGUF 解析硬界限见 §2.2；模型文件本身交给 llama.cpp 校验（坏文件 = 加载失败 = `Unavailable`，不 panic）。
- 零网络外呼：`aa4c-ai` crate 只连 localhost。模型下载走既有下载中心（用户显式发起）。
- 文件移动是唯一的"破坏性"操作：仅由确定性规则或用户手动触发（§0），全量 `archive_log` 可撤销。

## 8. UI（归档页，替换 UnderConstruction 占位）

顶层四个分区（tab 或分段卡片，沿用现有页面样式语言）：**归纳**（最近归档动作 + 待确认 AI 建议 + 撤销）、**规则**（列表/开关/编辑/排序 + 空态"一键启用推荐规则"）、**模型库**（§3.5）、**知识库**（来源管理 + 摄入进度 + 问答框，AI4 前隐藏或占位）。设置页新增「归档」区块（归档根目录 + 自动归档开关）与「AI」区块（模型目录、空闲自停分钟数）。文案禁术语（AGENTS.md）：不出现 GGUF/llama.cpp/RPC/embedding——用「模型文件」「本地 AI」「知识库索引」。

## 9. 里程碑与验收（详细步骤见 V0.5_IMPLEMENTATION_PLAN.md）

| 里程碑 | 内容 | 交付判定（真实环境，不是单测绿了就算） |
|--------|------|------|
| AI1 ✅ 已实现 | 文档先行 + 规则式归档（识别/GGUF/规则/移动/撤销/UI）| 真实下载一个小 .gguf → 自动移入模型目录、记录可撤销；同步范围警示可见 |
| AI2 | `aa4c-engine` 重构 + llama-server 接入 + 打包腿 + 模型库 | 真机加载真实模型 `/health` 就绪；三平台安装包含引擎且 AppImage 验证通过 |
| AI3 | AI 标签/分类建议（批量队列 + 待确认流） | 真实模型对一批真实文件出建议，采纳后标签/移动生效 |
| AI4 | 知识库（摄入/检索/流式问答） | 对自己的笔记目录问一个问题得到带引用的回答 |
| AI5 | 收尾：全量验证 + 文档 + `v0.5.0-preview` 发布 | 三平台 + Android APK 发布产物齐全 |

## 10. 已确认决策表

| 议题 | 决定 | 落点 |
|------|------|------|
| 自动 vs 建议 | **规则自动、AI 建议**；AI 输出永不直接驱动文件操作 | §0/§5/§6 |
| AI 形态 | llama.cpp `llama-server` sidecar（MIT），OpenAI 兼容 HTTP + 手写客户端；零云端 | §3 |
| 引擎分发 | 官方预编译产物（已实证三平台齐全）→ engines.yml 轻量腿裁剪转发布，**不自建源码构建** | §3.1/§3.4 |
| 密钥传递 | `LLAMA_API_KEY` 环境变量（已实证支持），不走命令行（rpc-secret 教训） | §3.2 |
| 生命周期 | 懒启动 + 空闲自停（默认 10 分钟），对话/嵌入两个独立槽位 | §3.3 |
| 预设规则默认态 | 全部**停用**，空态一键启用（不意外原则）；`archive_auto_enabled` 总闸默认开 | §2.3/§4 |
| 自动触发范围 | **仅下载完成**；同步/收件不自动归档（避免无人值守触发跨设备删除传播） | §2.4 |
| 移动语义 | rename→EXDEV 回退拷贝；更新 `download_tasks.save_path`；全量 log 可撤销 | §2.4 |
| 类别 vs 标签 | 类别内置固定，标签自由 | §2.1 |
| GGUF 解析 | 纯 Rust 手写只读头，硬界限防御，v2/v3 only | §2.2 |
| 向量检索 | SQLite BLOB + Rust 暴力余弦；不引向量库（个人规模） | §6 |
| 知识库范围 | 文本族 only；PDF/Office/多模态后置 | §6/§11 |
| AI 建议持久化 | 不落库（内存态，重启即清） | §4 |
| sidecar 设施 | 抽新 crate `aa4c-engine`，机械重构零行为变化 | §1 |
| 推荐模型 | Qwen3-4B-Instruct（对话）+ Qwen3-Embedding-0.6B（嵌入），双源直链（ModelScope+HF），不随包分发 | §3.5 |
| 平台 | 桌面三平台；Android/服务器后置 | §0 |

## 11. 实现期必须补的实证 + 仍待实现/后续

**AI2 动手前必须逐项实证（对照 D1"先核实再定案"教训）**：① 锁定的 llama.cpp tag 三平台产物内容与依赖闭包（`otool -L`/`ldd` 逐文件核对裁剪清单）；② `llama-server` JSON Schema 约束输出的请求字段形态；③ macOS 官方两架构包逐文件 `lipo` 后 universal 构建真实通过；④ AppImage 在 `-libs/` 进 `LD_LIBRARY_PATH` 后真实打包成功（真实 CI 跑，不猜）；⑤ 推荐模型的 ModelScope/HF 直链与确切文件名；⑥ CI 用微型 GGUF（如 tinyllamas stories260K，≈1 MB，MIT）做真实进程集成测试——**先把这个微型模型上传到我们自己的 `engines/test-fixtures` release** 再在测试里引用（不依赖第三方 URL 稳定性，engines.yml 惯例）；嵌入端点在非嵌入模型上需 `--pooling mean`，可用性一并实测。

**仍待实现/后续**：图片/视频的多模态识别（llama.cpp mtmd 已在产物里，等场景成熟）；PDF/Office 文本提取；GPU 变体（CUDA/Vulkan）与硬件检测；Android/服务器端 AI；标签检索进统一文件视图；知识库多轮对话；规则的更多占位符与条件；Lua 插件钩子对接归档动作（D4 之后）。

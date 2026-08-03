# AA4C 归档与 AI 设计（V0.5「AI」）

> 状态：**里程碑 AI1（规则式归档）、AI2（llama-server 引擎接入）、AI3（AI 标签/分类建议）、AI4（本地知识库）已实现**；AI5（收尾发布）仍是设计稿。对应 [ROADMAP.md](ROADMAP.md) V0.5（AI 归档：自动分类 / 标签 / 模型管理 / 本地知识库）；实现拆解见 [V0.5_IMPLEMENTATION_PLAN.md](V0.5_IMPLEMENTATION_PLAN.md)（里程碑 AI1–AI5）。
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

### 3.1 已实证的关键事实

**规划阶段（2026-07-21）粗验**：确认 llama.cpp 官方 release 对三平台都有预编译产物、产物形态是可执行文件+动态库、支持环境变量配置、MIT 协议——见下方"AI2.0 实现期实证"，规划阶段的 tag（`b10069`）已过期被替换，以实现期实测为准。

**AI2.0 实现期实证（2026-07-29，`aa4c-engine` 动手前，真机下载+运行验证）**：

1. **Tag 锁定 `b10175`**（规划阶段的 `b10069` 早已被上游新 tag 取代——llama.cpp 近乎每日发版，**实现期一律取当时最新 tag，不追规划期写死的旧 tag**）。四个目标平台产物 `llama-<tag>-bin-{macos-arm64,macos-x64,ubuntu-x64,win-cpu-x64}.{tar.gz,zip}` 全部下载校验通过（GitHub API `size` 字段逐字节比对，`curl -sSL --fail` 首次下载曾静默截断 3/4 个文件——**必须校验字节数，不能只看 curl 退出码**）。
2. **裁剪清单（`llama-server` 实际依赖闭包，`otool -L` / `objdump -p`（本机无 `readelf`，LLVM `objdump -p` 对 ELF `NEEDED`、PE `DLL Name` 均可读，够用）逐层实测）**：
   - **macOS（arm64 / x64 同形，x64 缺 `libggml-metal`）**：保留 `llama-server` + `libllama-server-impl.dylib` + `libllama-common.0.0.<ver>.dylib` + `libmtmd.0.0.<ver>.dylib` + `libllama.0.0.<ver>.dylib` + `libggml.0.17.0.dylib` + `libggml-cpu.0.17.0.dylib` + `libggml-blas.0.17.0.dylib` + `libggml-metal.0.17.0.dylib`（仅 arm64）+ `libggml-rpc.0.17.0.dylib` + `libggml-base.0.17.0.dylib` + `LICENSE`，**外加官方包里对应的 `libxxx.0.dylib → libxxx.0.17.0.dylib` 这批 rpath 版本号 symlink**（`llama-server` 的 `@rpath` NEEDED 条目写的是短版本名，不是全版本文件名，**漏掉 symlink 会导致加载失败**——`tar -tvzf`/`find -type l` 才能看到，普通 `find -type f` 会漏掉，这是本次实证过程中的真实踩坑）。其余 ~30 个 `llama-cli`/`llama-bench`/`llama-quantize`/…工具二进制及其专属 `libllama-*-impl.dylib` 一律丢弃。
   - **Linux（ubuntu-x64）**：保留 `llama-server` + `libllama-server-impl.so` + `libllama-common.so.0.0.<ver>` + `libmtmd.so.0.0.<ver>` + `libllama.so.0.0.<ver>` + `libggml.so.0.17.0` + `libggml-base.so.0.17.0` + `LICENSE`，**同样需要 `libxxx.so.0 → libxxx.so.0.0.<ver>` 这批 SONAME symlink**（`objdump -p` 里 `NEEDED` 写的正是 `libggml.so.0` 这种短名）。`libggml-cpu-*.so`（`alderlake`/`cascadelake`/`x64`/`zen4`…十余个按 CPU 微架构区分的变体）**不是链接期 NEEDED 依赖，是 ggml 后端注册表运行期按 CPU 特征探测后 dlopen 的插件**（`llama-server`/`libllama-server-impl.so` 的 NEEDED 列表里都没有它们）——**必须整批保留**（不能只留一个 baseline，否则老 CPU 探测不到对应变体会退化或失败）。`libggml-rpc.so`（分布式推理客户端后端）与 `ggml-rpc-server`（独立 RPC server 工具）本项目不启用 `--rpc`，**丢弃**。
     ⚠️ **系统依赖风险（新发现，AppImage 打包的真实隐患）**：`libllama-server-impl.so`/`libllama-common.so` 的 NEEDED 里有 `libssl.so.3`/`libcrypto.so.3`（OpenSSL 3.x）和 `libgomp.so.1`（OpenMP），**官方产物完全不带这两类库，指望目标系统已装**。CI/开发机（新版 Ubuntu）默认有，但老发行版可能只有 OpenSSL 1.1 没有 3.x，会导致 AppImage 在部分 Linux 发行版上启动即报"找不到共享库"。`libstdc++.so.6`/`libm.so.6`/`libgcc_s.so.1`/`libc.so.6`/`ld-linux-x86-64.so.2` 是 glibc/gcc 运行时，各发行版基本都有，风险低。**AI2.3 打包腿必须决定：要么在 `release.yml` 的 AppImage 里额外裁一份 `libssl`/`libgomp` 一并塞进 `-libs/`，要么在文档里明确声明"需要系统自带 OpenSSL 3.x + libgomp1"的最低发行版版本——这是本次新发现，规划阶段（§3.4）未预见，必须在 AI2.3 落地前拍板，不能拖到发版后才发现（参见本节 AppImage 预警的既有教训同类模式）**。
   - **Windows（win-cpu-x64）**：保留 `llama-server.exe` + `llama-server-impl.dll` + `llama-common.dll` + `mtmd.dll` + `llama.dll` + `ggml.dll` + `ggml-base.dll` + 全部 `ggml-cpu-*.dll`（与 Linux 同理，运行期按 CPU 特征加载，非链接期依赖，整批保留）+ `libomp140.x86_64.dll`（OpenMP 运行时，**Windows 没有系统级 libgomp 等价物，必须随包**）。`KERNEL32.dll`/`WS2_32.dll`/`CRYPT32.dll`/`MSVCP140.dll`/`VCRUNTIME140.dll`/`api-ms-win-crt-*.dll` 为系统 CRT/VC++ 运行时，按 D2 既有做法处理（Win10+ 系统自带或随 VC++ Redistributable，非本项目打包责任）。丢弃全部 `llama-*.exe`（除 `llama-server.exe`）及其专属 `*-impl.dll`。
3. **`llama-server` 参数环境变量确认**：`--help` 实测除规划阶段已知的 `LLAMA_ARG_MODEL`/`LLAMA_ARG_PORT`/`LLAMA_ARG_HOST`/`LLAMA_ARG_EMBEDDINGS`/`LLAMA_ARG_CTX_SIZE`/`LLAMA_ARG_THREADS`/`LLAMA_API_KEY` 外，**嵌入槽位必须加 `--pooling {none,mean,cls,last,rank}`（env: `LLAMA_ARG_POOLING`）**——用非专用嵌入模型（本次用 stories260K 验证）跑 `/v1/embeddings`，不带 `--embeddings --pooling mean` 会直接 501 `"This server does not support embeddings. Start it with --embeddings"`；带上两个参数后端到端跑通。
4. **JSON Schema 约束输出请求形态实测确认**：OpenAI 兼容形态 `response_format: {"type": "json_schema", "json_schema": {"name": "...", "schema": {...}}}`，随 `POST /v1/chat/completions` 请求体一起发送，服务端接受并按 schema 做 grammar 约束采样（本机用微型模型验证请求被正确处理、返回 200，不报 schema 格式错误；微型模型本身能力弱，生成内容不完全合规不代表请求形态错——**验证的是接口契约，不是模型质量**）。
5. **真实端到端跑通**（微型模型 `stories260K.gguf`，来源见下）：本机启动 `llama-server`（macOS arm64 二进制，`--no-webui`），`GET /health` → `{"status":"ok"}`；`POST /v1/chat/completions`（含带 `response_format` 的请求）→ 200，正常返回 `choices[0].message.content`；重启并加 `--embeddings --pooling mean` 后 `POST /v1/embeddings` → 200，返回 `data[0].embedding` 浮点数组。三个端点全部真实进程验证通过，非文档推断。
6. **微型 GGUF 测试固件来源**：`stories260K.gguf`（llama.cpp 官方 server 测试套件自用固件，来源见 `scripts/fetch_server_test_models.py` + `tools/server/tests/utils.py` 的 `model_hf_repo="ggml-org/models"`/`model_hf_file="tinyllamas/stories260K.gguf"`）。实测真实下载地址已从 `ggml-org/models` 302 跳转到 `ggml-org/models-moved`：`https://huggingface.co/ggml-org/models-moved/resolve/main/tinyllamas/stories260K.gguf`，大小 1,185,376 字节，SHA256 `270cba1bd5109f42d03350f60406024560464db173c0e387d91f0426d3bd256d`。**已上传到 `engines/test-fixtures` release**（含 `SHA256SUMS`，下载校验一致），CI 集成测试可直接引用，不依赖第三方 URL 稳定性。
7. llama.cpp 许可证 **MIT**——比 GPL 引擎更宽松，sidecar 进程隔离照旧（架构一致性，不是许可证被迫）。

### 3.2 进程与 RPC

- 启动：`llama-server` 由 `TauriSidecarSpawner`（桌面）/`ProcessSpawner`（测试/headless）拉起，配置全走环境变量：`LLAMA_ARG_HOST=127.0.0.1`、随机端口、随机 `LLAMA_API_KEY`（uuid 拼 32 字节 base58，同分享 token 生成法）、`LLAMA_ARG_MODEL=<模型路径>`、`LLAMA_ARG_CTX_SIZE=8192`；嵌入槽位额外 `LLAMA_ARG_EMBEDDINGS=1`。
- 孤儿防护：复用 `aa4c-engine::orphan_guard` 三平台路径（Windows Job Object / Linux `PR_SET_PDEATHSIG` / macOS PID 文件），llama-server 与 transmission-daemon 一样没有 `stop-with-process` 类机制。
- RPC：**手写极简 HTTP/1.1 客户端**（照抄 `TransmissionClient` 先例，不引 reqwest）。用的端点：`GET /health`（就绪门），`POST /v1/chat/completions`（OpenAI 形态；批量任务非流式、知识库问答走 SSE 流式——`data: {...}` 行解析，`[DONE]` 结尾），`POST /v1/embeddings`。鉴权 `Authorization: Bearer <key>`。
- **结构化输出**：分类/标签建议请求带 JSON Schema 约束（llama-server 支持 grammar 约束采样，输出保证合法 JSON，杜绝"解析模型自由发挥"这类脆弱代码）。请求字段形态已实证（§3.1 第 4 点）：`response_format: {"type": "json_schema", "json_schema": {"name", "schema"}}`。
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

### 3.6 AI2.1–AI2.4 实现偏差（2026-07-30，代码已落地，仅剩 AI2.5 全量验证）

- **`aa4c-engine` 重构（AI2.1）**：`SidecarSpawner`/`EngineChild`/`ProcessSpawner`/`orphan_guard` 已从 `aa4c-download` 平移到新 crate `crates/aa4c-engine`；`aa4c-download` 用 `pub use aa4c_engine::{...}` 重导出，`aa4c-core`/`apps/desktop` 的既有导入路径零改动；`orphan_guard` 内部原本的 `pub(crate)` 全部提升为 `pub`（现在要跨 crate 用）。纯机械重构，`cargo test --workspace` 全部既有测试零修改通过（判据达成）。
- **`SidecarSpawner` trait 新增 `envs` 参数（AI2.2 发现的必需扩展，不在原计划里）**：原 trait 只有 `spawn(&self, args: &[String])`，密钥/端口/模型路径这类 aria2/Transmission 不需要、但 llama-server **必须**通过环境变量传的动态值，没有地方能塞进去——Transmission 的库搜索路径是"壳层内部固定知道怎么算的静态信息"，`TauriSidecarSpawner` 一直是内部直接处理，从没走过参数；llama-server 反过来是"调用方每次 spawn 时才知道的动态值"，必须有一条参数通道。改成 `spawn(&self, args: &[String], envs: &[(String, String)])`，`ProcessSpawner`/`TauriSidecarSpawner` 两个实现与 `aa4c-download` 内的两个调用点（aria2/Transmission 都传 `&[]`，行为不变）一并更新，`cargo test --workspace` 复核过零回归。
- **`aa4c-ai` crate 已落地**：`LlamaClient`（手写 HTTP/1.1，§3.2 描述的三个端点 + 流式 `chat_completion_stream`）、`LlamaProcess`（拉起+120s 健康轮询+优雅退出，§3.2/§3.3）、`AiService`（懒启动+空闲自停双槽位，§3.3；`AiEngineState` 事件已加进 `aa4c_types::CoreEvent`，`event_name()` 为 `"ai_engine_state"`）。
- **`Connection: close` 是必需的，不是可选优化**：真机抓包发现 llama-server（cpp-httplib）默认走 HTTP keep-alive（不带这个请求头时响应带 `Keep-Alive: timeout=5, max=100`，连接不会自己关，D1/D2"读到 EOF = 读完"的假设不成立）；客户端主动发送 `Connection: close` 后，服务端会尊重它，**连流式（`Transfer-Encoding: chunked`）响应也会在结束后主动关闭 socket**——两种请求形态因此可以共用同一套"读到 EOF"心智，但流式响应必须先做 chunked 解码（不能像非流式那样囫囵读完整段）。
- **SSE 增量解析用了真正的增量式 chunked 解码器**，不是"整段收完再切"——真机抓包确认过：分块大小（`205`/`f8`/`1e3`…十六进制）与 TCP 单次 `read()` 的字节数没有任何关系，解码器必须能在任意字节边界被切开喂入仍然正确（`chunked_decoder_handles_split_across_arbitrary_byte_boundaries` 测试逐字节位置穷举验证过）。
- **空闲自停与"正在流式生成"之间的竞态**：`AiService` 的槽位大锁只在"确保进程已启动"这一步持有，实际推理请求期间不持锁（否则长请求会让并发请求、巡查任务全部卡住）——换来的代价是巡查任务判断"是否空闲"用的是一个独立的 `Arc<Mutex<Instant>>` 时间戳，流式请求的转发任务每收到一个 token 就刷新它一次，阻塞式请求在开始前/结束后各刷新一次。已知的剩余竞态窗口：一次阻塞式请求如果跑得比 idle_timeout 还长且中途没有任何输出（理论上不太可能，但没有硬性排除），巡查任务可能会在它进行中把进程杀掉——后果是这次请求收到连接被重置的错误，不会破坏其他状态，V0.5 默认 10 分钟空闲超时下判定为可接受，暂不做更复杂的忙碌引用计数。
- **测试**：12 个测试，含 5 个真实进程集成测试（真实 `llama-server` 二进制 + `stories260K.gguf`，覆盖健康检查、阻塞式 `/v1/chat/completions`、流式 SSE 多 chunk、`/v1/embeddings`、懒启动+空闲自动回收+PID 文件清理全链路）——真实二进制缺失时 `require_llama_server`/`require_tiny_model` 直接 panic 报安装指引，不静默跳过（同 `require_aria2c` 先例）。`cargo test --workspace`（含 `aa4c-core` 单线程复核）全绿。
- **打包腿（AI2.3）已完成，真实 CI 验证过（run 30518097243，2026-07-30）**：`engines.yml` 新增 `llama-macos`（arm64/x64 两个矩阵 job，各自下载官方产物+按 §3.1 第 2 点裁剪+隔离环境冒烟测试）/`llama-linux`（同样裁剪，额外自带 `libssl.so.3`/`libcrypto.so.3`/`libgomp.so.1`——采用"自己打包不依赖目标系统"的方案，§3.1 第 2 点的两个候选之一）/`llama-windows`/`llama-publish` 四个 job，全部真实跑通并发布到 `engines/llama-b10175` release（含 `SHA256SUMS`）。`scripts/fetch-engines.sh` 已接上真实校验和 + 下载/校验/解包逻辑（镜像 Transmission 的既有形态），`--from-path` 开发模式对 llama-server 是 best-effort（同 Transmission 先例，找不到只警告，不阻塞本地开发）。`tauri.conf.json` 加了第三个 `externalBin`（`binaries/llama-server`）+ `resources`（`binaries/llama-server-*-libs/*: llama-libs/`）；`download_spawner.rs` 的 `transmission_lib_search_env` 泛化成 `lib_search_env(app, resource_subdir)`，`llama-server` 复用同一套 `DYLD_LIBRARY_PATH`/`LD_LIBRARY_PATH` 注入机制（打包后 externalBin 与 resources 落在不同目录，rpath 假设"同目录"不成立，同 Transmission 的既有教训）。`ci.yml` 的 `test` job 现在三平台都装真实 `llama-server`（macOS `brew install llama.cpp`；Linux/Windows 下载官方产物）+ 下载真实 `stories260K.gguf`（校验 SHA256）+ 设置 `AA4C_TEST_LLAMA_SERVER_BIN`/`AA4C_TEST_TINY_GGUF`/`LD_LIBRARY_PATH`，让 `aa4c-ai` 的 5 个真实进程集成测试在 CI 里真正跑起来，不是本地专属。`release.yml` 加了 macOS 双架构 lipo 合并逻辑——**与 Transmission 不同，llama-server 的两个单架构 `-libs/` 目录不完全对称**（x64 官方产物没有 `libggml-metal`，Apple Silicon 专属的 GPU 后端），合并逻辑按文件名并集处理：两边都有就 lipo，只有一边有就原样拷贝。同时补了 `libssl3`/`libgomp1` 到 Linux AppImage 打包 job 的系统依赖列表（linuxdeploy 打包阶段的 ELF 扫描走系统路径，不认 `bundle.resources`，同 D2 AppImage 那次教训的直接推论）。
- **真机验证过程中发现并修的两个真问题**（不是纸面推演）：① 首次真实 CI 跑 `llama-macos`/`llama-linux`/`llama-windows` 三个 job 并发查询 `api.github.com` 做字节数校验，触发了未认证请求的匿名限额（403）——加 `Authorization: Bearer ${{ secrets.GITHUB_TOKEN }}` 解决；Windows 那条腿的 PowerShell 字符串插值 `"attempt $attempt: ..."` 因为 `$attempt:` 被解析成作用域限定变量引用报语法错，改成 `${attempt}:`。② 用真实下载的裁剪产物（未加 `zip -y`）在本机实测跑 `llama-server` 加载真实模型，健康检查要等 ~13 秒才通过（原始未裁剪产物 <1 秒）——根因是 `zip -r` 默认解引用 symlink，把每个 rpath 版本号 symlink 家族（1 个真文件 + 2 个 symlink）都变成 3 份独立文件，macOS 对每个新文件单独做一次 on-demand 签名校验，文件数从 9 变 27 拖慢了冷启动；`llama-publish` 的 `zip -r` 已经改成 `zip -ry`（保留 symlink），但这个修复**改动落地时机在第一次真实发布之后**，`engines/llama-b10175` release 当前挂着的资产仍是未加 `-y` 的版本（`gh run rerun --job` 复用的是触发时快照的 workflow 版本，不会自动捡起后续 push 的修复；byte-identical，functionally correct，只是冷启动稍慢，在 120s 健康检查预算内完全能接受）——**下次因任何原因重新跑一次 `engines.yml` 的 llama 腿（例如升级 tag）会自动带上这个修复**，不需要专门为了这一条再触发一次。
- **`ci.yml` 三平台真实验证过打包+集成测试链路（多轮真实 push，2026-07-30）**：macOS/Ubuntu/Windows 三个 `test` job 最终全部跑绿（过程中真机发现并修了三个真问题——Windows `sha256sum` 在含反斜杠路径下会给整行加转义前缀导致校验和比对总是失败；AI1 遗留的一个 Windows 路径分隔符测试 bug（`archive_root.join("模型/model.gguf")` 把字面反斜杠写死进单个 path 分量，Windows 上跟真实产出的 `\` 分隔路径对不上，两处测试断言改成 `.join("模型").join("model.gguf")`，与本次 AI2 工作无关但恰好被同一轮真实 CI 测出来一起修了）；`aa4c-ai` 一个测试在真实 CI 上（多个测试并发拉起真实 `llama-server` 抢 CPU）连续两次卡在同一个已知竞态窗口，idle_timeout/等待时间从最初的 500ms/3s 一路加宽到 10s/25s 才稳定）。`cargo check -p aa4c-desktop` 本地验证过：`llama-server` externalBin 缺失时正确报错阻断编译，补上后正常通过。
- **模型库（AI2.4）已完成**：`Settings` 新增 `ai_models_dir`/`ai_chat_model`/`ai_embedding_model`/`ai_idle_timeout_minutes`（`aa4c-types`+`aa4c-core` 的 load/save，镜像 `archive_root`/`archive_auto_enabled` 写法；`ai_models_dir` 默认值依赖 `archive_root`，`Core::start()` 里先算出 `archive_root` 再构造 `Settings` 字面量）。`Core::start()` 按 `config.ai_spawner` 是否注入决定要不要实例化 `AiService`（镜像 `DownloadService` 的可选能力接线），`Core::shutdown()` 一并停掉；`Core::update_settings()` 新增对 `ai_chat_model`/`ai_embedding_model` 变化的 diff，调用新增的 `AiService::set_model()`（换模型时顺手停掉正在跑的旧进程，不需要重启应用）。`AiService` 新增查询方法 `status(kind) -> aa4c_types::AiSlotStatus`（一次性快照，不经事件总线）。Command：`list_local_models`（递归一层扫描 `ai_models_dir`，读 GGUF 头，坏文件跳过不中断整批）、`get_ai_status`。桌面壳层 `desktop_download_spawner` 改名 `desktop_sidecar_spawner`（三个引擎共用同一个通用实现，旧名字带"download"是历史遗留，D1/D2 时只有下载引擎）。前端：新 `LocalModel`/`AiStatus`/`AiSlotStatus`/`AiEngineStatePayload` 类型 + `useAiStore` + `aa4c://ai_engine_state` 事件桥接；设置页新增「AI」区块（模型目录选择器+空闲超时分钟数）；归档页新增「模型库」分区（列表 + 设为对话/嵌入模型按钮 + 引擎运行状态提示）。**推荐模型双源直链（ModelScope/HF）当时未做**——见 §11 AI5 补记，实现期最终在 AI5 收尾时核实补上。`pnpm build` 通过，浏览器走查过两个新区块渲染正常（无 Tauri 后端的纯前端 dev 模式下优雅显示空态，不报错）。
- **未做**：AI2.5（全量验证 + 文档收尾）；`release.yml` 的 AppImage 产出与 macOS lipo 合并逻辑本身**尚未被真实 tag push 验证过**（只验证到 `cargo check -p aa4c-desktop` 本地通过 + `engines.yml` 独立跑通，release.yml 要等真实打 tag 发版才会触发，按计划这是 AI5 的事）；真机走查 `pnpm tauri dev`（当前只验证了纯前端 dev 模式，没有真实 Tauri 后端+真实模型的端到端点击走查）。

### 3.7 AI3 实现偏差（2026-08-02，代码已落地，全量验证通过）

- **`aa4c-ai::suggest` 模块**：`SuggestEngine`（单并发批量队列 + 门闩，重叠批量直接拒绝而不是排队——§5 描述的场景本来就是"等一批做完再发下一批"，不需要队列语义）；`build_request`/`parse_suggestion` 严格照抄 AI2.0 实证的请求形态（`response_format.json_schema`，见 §3.1 第 4 点）。`aa4c-ai` 依旧不碰文件系统——`SuggestInput`（path/category/size/text_head）由调用方（`aa4c-core::orchestrate::start_suggest`）组好传入，`text_head` 只对 Document/Code/Subtitle 三个类别读（图片/视频/音频/模型/压缩包/安装包是二进制，读了也是乱码），读取用 `Read::take` 限流到 8KB 而不是整读再截断。
- **`Core` 编排**：`start_suggest`/`list_suggestions`/`resolve_suggestion` 与设计一致；`resolve_suggestion(id, adopt, target_dir)` 内部新增 `archive::engine::apply_suggestion()`（`finish_move` 抽出的 `record_entry` 共享辅助函数，供规则式归档与 AI 建议采纳共用"落 `archive_entries` + 打标签"这一步）——采纳时 `target_dir` 给了才移动文件+写 `archive_log`+广播 `ArchiveApplied`，不给就原地打标签（`TagSource::Ai`），不生成可撤销的日志记录（没有物理移动，撤销无意义）。建议的 `category` 直接采用模型输出，不重新跑 `detect_category` 覆盖——用户点"采纳"就是认可了这个类别。
- **UI**：沿用 AI1 建立的先例——归档页仍是纵向堆叠的独立 `<h3>` 分区，不是 §8 描述的 tab/分段卡片；"AI 建议"是独立分区（进度 + 待确认列表 + 采纳/忽略按钮），未与"最近动作"合并成单一"归纳"分区。对话模型未配置时按钮禁用并提示去模型库选一个，不允许发起注定失败的请求。
- **测试**：`aa4c-ai::suggest` 单元测试覆盖请求构造/响应解析/未配置引擎降级/重叠批量拒绝，另有 1 个真实 `llama-server`+微型 GGUF 的端到端测试（对真实文件出建议，只断言 schema 合法，不断言内容质量——微型模型说胡话是预期内的失败模式）；`aa4c-core` 新增 `ai_suggest_lifecycle_through_core_orchestration`，走 `Core` 公开方法用真实模型跑通"选文件 → 出建议 → 采纳 → 文件真的被移动/打标签/可撤销"全链路。`cargo test --workspace`（真实模型环境变量注入）全绿，含单线程复核排除并行资源争抢导致的已知 QUIC flaky 干扰（`quic_resume_after_disconnect`/`two_cores_pair_then_transfer` 隔离重跑均通过，与本次改动无关）。

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

### 6.1 AI4 实现偏差（2026-08-03，代码已落地，全量验证通过）

- **crate 归属完全按 §1 设计落地**：`aa4c-ai::kb`（新子模块 `kb/mod.rs` + `kb/chunk.rs`）直接依赖 `aa4c-store`（不经 `aa4c-core` 中转），扫描/分块/嵌入/CRUD/检索/问答整条流水线都在这一个 crate 里——`aa4c-store` 只依赖 `aa4c-types`，这条新依赖边不会与既有的 `aa4c-types ← aa4c-engine ← (aa4c-download, aa4c-ai) ← aa4c-core` 依赖链形成环。`aa4c-core::orchestrate` 只做 5 个 Command 的薄转发（`kb_add_source`/`kb_remove_source`/`kb_list_sources`/`kb_reindex`/`kb_ask`），不重复任何逻辑。
- **来源目录不预填候选**：§6 提到"天然候选：归档根、同步范围"，实现期未做——用户通过目录选择器任意选目录，没有一键"用归档根"这类快捷入口。不是遗漏，是先做最小可用版，候选快捷方式留给以后有需要时再加。
- **"5 万 chunk 起提示知识库偏大"的 UI 警告未做**：`aa4c-store::count_kb_chunks`/`KbService::total_chunks` 已经实现（后续要接这个警告随时能接），但归档页目前没有读它来显示警告文案。个人规模场景下（数千到数万 chunk）不太可能现在就撞到这个阈值，暂不视为阻塞项。
- **"可随时中断"落地为结构性保证，不是一个显式取消 Command**：`replace_kb_chunks` 整个文档的 chunk 替换包在一个 SQLite 事务里，任何时刻应用退出/引擎故障，最坏情况是"最后一个文档没摄入成功"（下次 `kb_reindex` 会重试），不会出现某个文档只写了一半 chunk 的中间态。没有实现一个专门的"停止摄入"按钮/Command——5 个 Command 的范围以外，且摄入的自然完成时间（个人规模文档）不长，不构成实际痛点。
- **实现期发现的真问题：流式回答必须设 `max_tokens` 上限**——`run_ask`最初没限制生成长度，写 Core 级集成测试时一次真实调用触发了数百条 `KbAnswerDelta` 事件，测试用固定 200 次轮询上限读事件，在真正等到 `KbAnswerDone` 之前就把预算耗尽而失败（不是功能 bug，是"轮询次数固定"这个测试写法本身的假设不成立——广播事件可能瞬间挤爆缓冲区，不需要等 500ms 超时逐条读）。修法两处：① `run_ask` 的请求体加 `"max_tokens": 512`（防止跑题/没触发停止条件时无限生成，问答场景的合理上限，这本身也是生产代码该有的防御，不只是测试变快）；② 两个真实模型集成测试的轮询循环从"固定 `for _ in 0..N`"改成"墙钟时间预算 `while Instant::now() < deadline`"，能应对任意速率的事件突发。
- **测试**：`aa4c-ai::kb::chunk` 5 个纯函数分块单测（含超长单段落滑窗切分、段落边界重叠验证）；`aa4c-ai::kb` 单测覆盖余弦相似度（含零向量/维度不匹配）、top-k 排序截断、真实临时目录的扫描逻辑（含隐藏目录/`node_modules`/二进制文件排除）；另有 2 个真实 `llama-server`+微型 GGUF 端到端测试（`aa4c-ai::kb` 一个 + `aa4c-core::tests::core.rs` 新增的 `kb_lifecycle_through_core_orchestration` 一个，后者走 `Core` 公开方法验证"添加来源→摄入→问答→引用命中正确文件→删除来源"全链路，只断言 schema/流程完整，不断言回答内容质量——同 AI3 既有先例，微型模型说胡话是预期内的失败模式）。`cargo test --workspace`（真实模型环境变量注入）除已知的 `quic_resume_after_disconnect` 并行资源争抢 flaky（隔离重跑通过，与本次改动无关）外全绿；`cargo clippy --workspace --all-targets -- -D warnings`/`cargo fmt --all --check`/`pnpm build` 全部通过。

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
| AI2 ✅ 代码/CI 已实现，AppImage 打包待发版验证 | `aa4c-engine` 重构 + llama-server 接入 + 打包腿 + 模型库 | 真机加载真实模型 `/health` 就绪（已达成）；三平台安装包含引擎且 AppImage 验证通过（`release.yml` 只在真实 tag push 时触发，代码已就绪，端到端效果留给 AI5 发版时验证，见 §3.6） |
| AI3 ✅ 已实现 | AI 标签/分类建议（批量队列 + 待确认流） | 真实模型对一批真实文件出建议，采纳后标签/移动生效（已用真实 llama-server + 微型 GGUF 走通 `Core` 全链路） |
| AI4 ✅ 已实现 | 知识库（摄入/检索/流式问答） | 对自己的笔记目录问一个问题得到带引用的回答（已用真实 llama-server + 微型 GGUF 走通 `Core` 全链路：摄入→检索→流式回答→引用命中正确文件） |
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

**AI2.0 实证结论（2026-07-29，详见 §3.1）**：① ✅ tag 锁 `b10175`，三平台产物内容与依赖闭包逐文件核对，裁剪清单已写入 §3.1 第 2 点（含 macOS/Linux 的 rpath/SONAME symlink 陷阱、Linux 的 CPU 后端插件不可裁剪、Linux 的 `libssl`/`libgomp` 系统依赖新风险）；② ✅ JSON Schema 约束请求字段形态 `response_format.json_schema`，实测确认；③ **未验证**——本次只逐平台官方包核实，两架构 `lipo` 合并 universal 尚未实操，留给 AI2.3 打包腿动手时做（届时是真正需要产出 universal 二进制的时刻，规划阶段/AI2.0 均为核实产物形态，非打包实操）；④ **未验证**——同上，AppImage 真实打包留给 AI2.3+ 真实 CI，AI2.0 只发现并记录了 `libssl`/`libgomp` 这一新增系统依赖风险；⑤ ✅ **AI5 补验证**——推荐模型 ModelScope/HF 直链已核实（详见下方"AI5 补记"）；⑥ ✅ 微型 GGUF `stories260K.gguf`（来源、大小、SHA256 见 §3.1 第 6 点）已下载并本机真实跑通 `/health`+`/v1/chat/completions`（含 `response_format`）+`/v1/embeddings`（`--pooling mean`），并已上传到 `engines/test-fixtures` release。

**仍待实现/后续**：图片/视频的多模态识别（llama.cpp mtmd 已在产物里，等场景成熟）；PDF/Office 文本提取；GPU 变体（CUDA/Vulkan）与硬件检测；Android/服务器端 AI；标签检索进统一文件视图；知识库多轮对话；规则的更多占位符与条件；Lua 插件钩子对接归档动作（D4 之后）。

**AI5 补记（2026-08-03）——推荐模型直链核实**：对话模型用 `unsloth/Qwen3-4B-Instruct-2507-GGUF` 的 `Qwen3-4B-Instruct-2507-Q4_K_M.gguf`（Qwen 官方未发布该模型的 GGUF 量化，unsloth 是 llama.cpp 生态里认可度较高的第三方量化团队，HF 与 ModelScope 两边都有同名文件，`curl -I` 实测 HF 返回 302 到真实签名 CDN、ModelScope 返回 200，文件大小 2,497,281,120 字节≈2.5GB，与设计估算一致）；嵌入模型用 `Qwen/Qwen3-Embedding-0.6B-GGUF` 的 `Qwen3-Embedding-0.6B-Q8_0.gguf`（**这个是 Qwen 官方自己发布的 GGUF**，HF 与 ModelScope 都有同一账号同名仓库，文件大小 639,150,592 字节≈0.6GB）。四个直链已写入 `apps/desktop/src/pages/ArchivePage.vue` 的 `RECOMMENDED_MODELS`，点击后直接调用既有 `add_download`（V0.4 下载中心）把 URL 交给 aria2，文件名由服务端 `Content-Disposition` 决定；下载完成后走 AI1 的下载完成钩子自动归档进模型目录——不需要任何新后端代码，纯前端接线即可满足 DoD 第 6 条。

**AI5 补记（2026-08-03）——`v0.5.0-preview` 首次真实发布踩坑**：§3.6 提到的 macOS 双架构 lipo 合并逻辑（"两边都有就 lipo，只有一边有就原样拷贝"）**遗漏了一种情况**——两边都有、但根本不是 Mach-O 二进制的文件。llama.cpp 官方 release 包的 `-libs/` 目录里除了 dylib 还带了一份 `LICENSE` 纯文本文件，两个架构下内容相同，直接对着它 `lipo -create` 报 `can't figure out the architecture type`，中断整个 macOS 打包 job（`release.yml` run 30792608845 首次真实触发时发现，Windows/Ubuntu 两条腿不受影响先跑完，导致 GitHub Release 一度只有 Windows 两个产物）。修法：合并前先用 `lipo -info` 探测"两边都有的文件是不是真的 Mach-O"，不是的话（文本文件等）直接当"内容天然一致"处理，只拷贝一份，不试图合并——不再假设"文件名相同=一定是需要合并的二进制"。删除了那次不完整的 `v0.5.0-preview` release（仅 2 个 Windows 资产，0 下载）和对应 tag，修复后重新打 tag 触发全新一轮构建。

**AI5 补记（2026-08-03）——第二次真实发布踩坑：Linux AppImage 打包 + llama-server 资源库**：修完 macOS lipo 后重新触发（run 30794168004），macOS/Windows 都过了，但 Ubuntu 腿在 `tauri-action` 步骤又报了一次同一句语焉不详的 `failed to bundle project 'failed to run linuxdeploy'`——同 D2 时代 Transmission 那次一模一样的错误文案，但这次根因不同。§3.4 规划阶段就预警过这个风险："llama-server 的 libggml/libllama 依赖是我们自己的资源库，不是 apt 可装的，release.yml 的 Ubuntu job 必须在 tauri build 运行前注入 LD_LIBRARY_PATH，这一点必须真实 CI 验证，不能假设"——这次就是这个预警第一次真实复现：AI2.3 打包腿当时给 `apt-get install` 加了 `libssl3`/`libgomp1`（llama-server 二进制自身的系统级依赖），但 `libggml*.so`/`libllama.so` 这两个库本身从来没有对应的 apt 包能把它们装到系统路径——linuxdeploy 打包阶段扫描 `usr/bin/` 下每个可执行文件的 ELF 依赖时，走的是系统路径解析，找不到这两个纯资源库天经地义。修法：给 `tauri-action` 步骤本身注入 `LD_LIBRARY_PATH` 指向 `binaries/llama-server-x86_64-unknown-linux-gnu-libs`（同运行时 `download_spawner.rs` 给 llama-server 子进程注入的是同一个目录，只是这次服务的是打包阶段的 linuxdeploy 进程，不是运行阶段的子进程，两处注入职责不同、互不冲突）。同时给三个平台的构建参数都加了 `--verbose`——这是本次事故的直接教训：Tauri 默认不透出 linuxdeploy 的底层 stderr，报错文案两次事故（D2 的 Transmission、这次的 llama-server）完全一样、但根因完全不同，靠"文案匹配历史教训"猜根因不可靠，`--verbose` 从下一次开始默认打开，不用先触发一次失败才想起来加。

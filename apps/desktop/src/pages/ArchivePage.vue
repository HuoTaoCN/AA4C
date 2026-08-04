<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import { openPath } from "@tauri-apps/plugin-opener";
import { open } from "@tauri-apps/plugin-dialog";
import TabBar from "../components/TabBar.vue";
import { useAiStore } from "../stores/ai";
import { useArchiveStore } from "../stores/archive";
import { useDownloadStore } from "../stores/download";
import { useKbStore } from "../stores/kb";
import { useSettingsStore } from "../stores/settings";
import { useToastStore } from "../stores/toast";
import { asCommandError } from "../lib/api";
import { timeText, baseName } from "../lib/format";
import type { ArchiveCategory, ArchiveRule, LocalModel } from "../lib/types";

const archive = useArchiveStore();
const ai = useAiStore();
const kb = useKbStore();
const download = useDownloadStore();
const settings = useSettingsStore();
const toast = useToastStore();

onMounted(() => {
  void archive.loadAll();
  void ai.loadAll();
  void archive.loadSuggestions();
  void kb.loadSources();
});

// —— 页内分区（原先 5 个纵向堆叠区块合成一页太长，按用户任务拆成 3 组）——
const ARCHIVE_TABS = [
  { key: "rules", label: "规则与记录" },
  { key: "suggestions", label: "AI 建议" },
  { key: "models", label: "模型与知识库" },
];
const activeTab = ref<"rules" | "suggestions" | "models">("rules");

const CATEGORY_LABELS: Record<ArchiveCategory, string> = {
  model: "模型",
  image: "图片",
  video: "视频",
  audio: "音频",
  document: "文档",
  ebook: "电子书",
  archive: "压缩包",
  installer: "安装包",
  code: "代码",
  subtitle: "字幕",
  other: "其他",
};
const ALL_CATEGORIES = Object.keys(CATEGORY_LABELS) as ArchiveCategory[];

function ruleName(ruleId: string | null): string {
  if (!ruleId) return "手动";
  return archive.rules.find((r) => r.id === ruleId)?.name ?? "已删除的规则";
}

async function toggleRule(rule: ArchiveRule) {
  try {
    await archive.saveRule({ ...rule, enabled: !rule.enabled });
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}

async function saveTemplate(rule: ArchiveRule, template: string) {
  if (template === rule.action.targetTemplate) return;
  try {
    await archive.saveRule({
      ...rule,
      action: { ...rule.action, targetTemplate: template },
    });
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}

async function saveTags(rule: ArchiveRule, tagsText: string) {
  const tags = tagsText
    .split(",")
    .map((t) => t.trim())
    .filter(Boolean);
  try {
    await archive.saveRule({ ...rule, action: { ...rule.action, tags } });
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}

async function deleteRule(id: string) {
  try {
    await archive.deleteRule(id);
    toast.push("info", "已删除规则");
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}

const allDisabled = computed(
  () => archive.rules.length > 0 && archive.rules.every((r) => !r.enabled),
);
async function enableAllRecommended() {
  try {
    await Promise.all(archive.rules.map((r) => archive.saveRule({ ...r, enabled: true })));
    toast.push("success", "已启用全部推荐规则");
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}

// —— 新建规则（表单收起/展开）——
const showNewRule = ref(false);
const newRule = reactive({
  name: "",
  categories: [] as ArchiveCategory[],
  targetTemplate: "",
  tags: "",
});
function resetNewRule() {
  newRule.name = "";
  newRule.categories = [];
  newRule.targetTemplate = "";
  newRule.tags = "";
  showNewRule.value = false;
}
async function createRule() {
  if (!newRule.name.trim() || !newRule.targetTemplate.trim() || newRule.categories.length === 0) {
    toast.push("error", "名称、类别、目标目录都要填");
    return;
  }
  try {
    await archive.saveRule({
      id: "",
      name: newRule.name.trim(),
      enabled: true,
      position: archive.rules.length,
      matcher: {
        categories: newRule.categories,
        extensions: null,
        glob: null,
        minSize: null,
        maxSize: null,
      },
      action: {
        targetTemplate: newRule.targetTemplate.trim(),
        tags: newRule.tags
          .split(",")
          .map((t) => t.trim())
          .filter(Boolean),
      },
      createdAt: 0,
      updatedAt: 0,
    });
    toast.push("success", "已新建规则");
    resetNewRule();
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}

async function undoLog(logId: number) {
  try {
    await archive.undo(logId);
    toast.push("success", "已撤销");
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}

async function openEntryFolder(path: string) {
  await openPath(path);
}

const recentLog = computed(() => archive.log.filter((l) => !l.undone).slice(0, 20));

// —— 模型库（里程碑 AI2.4，ARCHIVE_DESIGN.md §3.5）——

function modelFileName(path: string): string {
  return baseName(path);
}
function modelSummary(model: LocalModel): string {
  const parts = [
    model.meta.architecture,
    model.meta.sizeLabel,
    model.meta.fileType,
  ].filter((p): p is string => !!p);
  if (model.meta.contextLength) {
    parts.push(`${model.meta.contextLength} ctx`);
  }
  return parts.length ? parts.join(" · ") : "未知格式";
}

async function selectModel(kind: "chat" | "embedding", path: string) {
  if (!settings.settings) return;
  try {
    const next =
      kind === "chat"
        ? { ...settings.settings, aiChatModel: path }
        : { ...settings.settings, aiEmbeddingModel: path };
    await settings.save(next);
    await ai.loadStatus();
    toast.push("success", kind === "chat" ? "已设为对话模型" : "已设为知识库模型");
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}

function isSelected(kind: "chat" | "embedding", path: string): boolean {
  const cur = kind === "chat" ? settings.settings?.aiChatModel : settings.settings?.aiEmbeddingModel;
  return cur === path;
}

/** 推荐模型直链（ARCHIVE_DESIGN.md §3.5）：URL 已实测核实可下载（HTTP 200/302 直连
 * 官方 CDN，见提交历史），文件名由服务端 `Content-Disposition` 给出，下载中心/aria2
 * 会按它落盘——不需要在这里手动拼文件名。 */
const RECOMMENDED_MODELS = [
  {
    key: "chat",
    label: "对话模型 · Qwen3-4B-Instruct-2507（Q4_K_M，约 2.5GB，8GB 内存可跑）",
    hfUrl:
      "https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/main/Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
    msUrl:
      "https://modelscope.cn/models/unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/master/Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
  },
  {
    key: "embedding",
    label: "知识库模型 · Qwen3-Embedding-0.6B（Q8_0，约 0.6GB，中英双强）",
    hfUrl:
      "https://huggingface.co/Qwen/Qwen3-Embedding-0.6B-GGUF/resolve/main/Qwen3-Embedding-0.6B-Q8_0.gguf",
    msUrl:
      "https://modelscope.cn/models/Qwen/Qwen3-Embedding-0.6B-GGUF/resolve/master/Qwen3-Embedding-0.6B-Q8_0.gguf",
  },
];

async function downloadRecommendedModel(url: string) {
  try {
    await download.add(url);
    toast.push("success", "已加入下载任务，完成后会自动归档到模型目录");
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}

// —— AI 建议（里程碑 AI3，ARCHIVE_DESIGN.md §5）——

async function pickFilesForSuggest() {
  const picked = await open({ multiple: true });
  if (!picked) return;
  const paths = Array.isArray(picked) ? picked : [picked];
  if (paths.length === 0) return;
  try {
    await archive.startSuggest(paths);
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}

async function adoptSuggestion(id: string) {
  try {
    await archive.resolveSuggestion(id, true);
    toast.push("success", "已采纳建议");
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}

async function ignoreSuggestion(id: string) {
  try {
    await archive.resolveSuggestion(id, false);
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}

// —— 本地知识库（里程碑 AI4，ARCHIVE_DESIGN.md §6）——

async function pickKbSourceDir() {
  const picked = await open({ directory: true });
  if (!picked || Array.isArray(picked)) return;
  try {
    await kb.addSource(picked);
    toast.push("success", "已添加来源");
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}

async function removeKbSource(id: string) {
  try {
    await kb.removeSource(id);
    toast.push("info", "已移除来源");
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}

async function reindexKbSource(id: string) {
  try {
    await kb.reindex(id);
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}

const kbQuestion = ref("");
async function askKb() {
  const question = kbQuestion.value.trim();
  if (!question) return;
  try {
    await kb.ask(question);
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}

async function openKbSourcePath(path: string) {
  await openPath(path);
}
</script>

<template>
  <div class="archive">
    <h2>归档</h2>
    <p class="intro muted">
      按规则自动把下载完成的文件分类归位（模型、图片、文档……）；AI 建议辅助打标签/分类，本地知识库可以对自己的文件提问。
    </p>

    <TabBar v-model="activeTab" :tabs="ARCHIVE_TABS" />

    <section v-show="activeTab === 'rules'">
      <h3>最近动作</h3>
      <div v-if="recentLog.length" class="card list">
        <div v-for="entry in recentLog" :key="entry.id" class="lrow">
          <div class="linfo">
            <div class="lname">{{ baseName(entry.toPath) }}</div>
            <div class="lmeta muted">
              {{ ruleName(entry.ruleId) }} · {{ timeText(entry.at) }}
            </div>
          </div>
          <button class="btn btn-ghost small" @click="openEntryFolder(entry.toPath)">
            打开位置
          </button>
          <button class="btn btn-ghost small" @click="undoLog(entry.id)">撤销</button>
        </div>
      </div>
      <div v-else class="empty card muted">还没有归档动作。</div>

      <h3>规则</h3>
      <p class="hint muted">
        按顺序取第一条命中的启用规则；新装规则默认停用，需要手动打开或一键启用。
      </p>
      <div v-if="allDisabled" class="card banner">
        <span>还没有启用任何归档规则，下载完成后不会自动归档。</span>
        <button class="btn btn-primary small" @click="enableAllRecommended">
          一键启用推荐规则
        </button>
      </div>

      <div v-if="archive.rules.length" class="card list">
        <div v-for="rule in archive.rules" :key="rule.id" class="rrow">
          <label class="switch small">
            <input type="checkbox" :checked="rule.enabled" @change="toggleRule(rule)" />
            <span class="slider"></span>
          </label>
          <div class="rinfo">
            <div class="rname">
              {{ rule.name }}
              <span class="cats muted">
                {{ rule.matcher.categories.map((c) => CATEGORY_LABELS[c]).join("、") }}
              </span>
            </div>
            <input
              class="rtemplate"
              type="text"
              :value="rule.action.targetTemplate"
              placeholder="目标目录模板，如 {类别}/{年}/{月}"
              @change="saveTemplate(rule, ($event.target as HTMLInputElement).value)"
            />
            <input
              class="rtags"
              type="text"
              :value="rule.action.tags.join(', ')"
              placeholder="标签（逗号分隔）"
              @change="saveTags(rule, ($event.target as HTMLInputElement).value)"
            />
          </div>
          <button class="btn btn-danger small" @click="deleteRule(rule.id)">删除</button>
        </div>
      </div>
      <div v-else class="empty card muted">还没有归档规则。</div>

      <div v-if="!showNewRule" class="new-rule-toggle">
        <button class="btn btn-ghost small" @click="showNewRule = true">+ 新建规则</button>
      </div>
      <div v-else class="card form">
        <div class="field">
          <label>规则名称</label>
          <input v-model="newRule.name" type="text" placeholder="例如：字幕归位" />
        </div>
        <div class="field">
          <label>匹配类别</label>
          <div class="cat-checks">
            <label v-for="c in ALL_CATEGORIES" :key="c" class="cat-check">
              <input type="checkbox" :value="c" v-model="newRule.categories" />
              {{ CATEGORY_LABELS[c] }}
            </label>
          </div>
        </div>
        <div class="field">
          <label>目标目录模板</label>
          <input v-model="newRule.targetTemplate" type="text" placeholder="{类别}/{年}/{月}" />
        </div>
        <div class="field">
          <label>标签（逗号分隔，可留空）</label>
          <input v-model="newRule.tags" type="text" placeholder="如：字幕" />
        </div>
        <div class="actions">
          <button class="btn btn-ghost small" @click="resetNewRule">取消</button>
          <button class="btn btn-primary small" @click="createRule">新建</button>
        </div>
      </div>
    </section>

    <section v-show="activeTab === 'suggestions'">
      <p class="hint muted">
        选几个文件让对话模型给出分类/标签建议——建议只会进这个待确认列表，不会自己改动文件（规则自动、AI 建议）。
      </p>
      <div class="card banner">
        <span v-if="archive.suggestRunning">
          正在生成建议…（{{ archive.suggestDone }}/{{ archive.suggestTotal }}）
        </span>
        <span v-else-if="!ai.status?.chat.configured">
          还没有配置对话模型——先在「模型与知识库」选一个。
        </span>
        <span v-else>选中文件，AI 会给出分类和标签，采纳前不会移动任何文件。</span>
        <button
          class="btn btn-primary small"
          :disabled="archive.suggestRunning || !ai.status?.chat.configured"
          @click="pickFilesForSuggest"
        >
          选择文件生成建议
        </button>
      </div>
      <div v-if="archive.suggestions.length" class="card list">
        <div v-for="s in archive.suggestions" :key="s.id" class="srow">
          <div class="sinfo">
            <div class="sname">{{ baseName(s.path) }}</div>
            <div v-if="s.error" class="smeta error">建议失败：{{ s.error }}</div>
            <div v-else class="smeta muted">
              {{ CATEGORY_LABELS[s.category] }}
              <template v-if="s.tags.length"> · {{ s.tags.join("、") }}</template>
              <template v-if="s.reason"> · {{ s.reason }}</template>
            </div>
          </div>
          <button
            class="btn btn-primary small"
            :disabled="!!s.error"
            @click="adoptSuggestion(s.id)"
          >
            采纳
          </button>
          <button class="btn btn-ghost small" @click="ignoreSuggestion(s.id)">忽略</button>
        </div>
      </div>
      <div v-else-if="!archive.suggestRunning" class="empty card muted">还没有待确认的建议。</div>
    </section>

    <section v-show="activeTab === 'models'">
      <h3>模型库</h3>
      <p class="hint muted">
        扫描模型目录（设置页可更改）下的模型文件；下载的模型经归档规则移入这里后会自动出现。
      </p>
      <div class="card list rec-list">
        <div v-for="rec in RECOMMENDED_MODELS" :key="rec.key" class="krow">
          <div class="kinfo">
            <div class="kname">{{ rec.label }}</div>
          </div>
          <button class="btn btn-ghost small" @click="downloadRecommendedModel(rec.hfUrl)">
            HF 下载
          </button>
          <button class="btn btn-ghost small" @click="downloadRecommendedModel(rec.msUrl)">
            ModelScope 下载
          </button>
        </div>
      </div>
      <div v-if="ai.models.length" class="card list">
        <div v-for="model in ai.models" :key="model.path" class="mrow">
          <div class="minfo">
            <div class="mname">{{ modelFileName(model.path) }}</div>
            <div class="mmeta muted">{{ modelSummary(model) }}</div>
          </div>
          <button
            class="btn small"
            :class="isSelected('chat', model.path) ? 'btn-primary' : 'btn-ghost'"
            @click="selectModel('chat', model.path)"
          >
            {{ isSelected("chat", model.path) ? "对话模型 ✓" : "设为对话模型" }}
          </button>
          <button
            class="btn small"
            :class="isSelected('embedding', model.path) ? 'btn-primary' : 'btn-ghost'"
            @click="selectModel('embedding', model.path)"
          >
            {{ isSelected("embedding", model.path) ? "知识库模型 ✓" : "设为知识库模型" }}
          </button>
        </div>
      </div>
      <div v-else class="empty card muted">
        还没有模型文件——下载一个模型，归档后会出现在这里。
      </div>
      <p v-if="ai.status" class="hint muted">
        对话引擎：{{ ai.status.chat.running ? "运行中" : ai.status.chat.configured ? "待命" : "未配置" }}
        · 知识库引擎：{{ ai.status.embedding.running ? "运行中" : ai.status.embedding.configured ? "待命" : "未配置" }}
      </p>

      <h3>知识库</h3>
      <p class="hint muted">
        添加一个文本文件目录作为来源，摄入后可以直接提问——回答只依据摄入的内容，并会带上参考的文件。
      </p>
      <div class="card banner">
        <span v-if="kb.activeIngest">
          正在摄入…（{{ kb.activeIngest.done }}/{{ kb.activeIngest.total }}）
        </span>
        <span v-else-if="!ai.status?.embedding.configured">
          还没有配置知识库模型——先在上面选一个。
        </span>
        <span v-else>选一个目录作为知识库来源，会扫描其中的文本/代码文件。</span>
        <button
          class="btn btn-primary small"
          :disabled="!ai.status?.embedding.configured"
          @click="pickKbSourceDir"
        >
          添加来源
        </button>
      </div>
      <div v-if="kb.sources.length" class="card list">
        <div v-for="source in kb.sources" :key="source.id" class="krow">
          <div class="kinfo">
            <div class="kname">{{ source.path }}</div>
            <div class="kmeta muted">
              已索引 {{ source.indexedCount }} / {{ source.docCount }}
              <template v-if="source.failedCount"> · 失败 {{ source.failedCount }}</template>
            </div>
          </div>
          <button
            class="btn btn-ghost small"
            :disabled="!!kb.activeIngest"
            @click="reindexKbSource(source.id)"
          >
            摄入
          </button>
          <button class="btn btn-ghost small" @click="openKbSourcePath(source.path)">
            打开位置
          </button>
          <button class="btn btn-danger small" @click="removeKbSource(source.id)">删除</button>
        </div>
      </div>
      <div v-else class="empty card muted">还没有知识库来源。</div>

      <div class="card kb-ask">
        <div class="ask-row">
          <input
            v-model="kbQuestion"
            type="text"
            placeholder="向知识库提问……"
            @keyup.enter="askKb"
          />
          <button
            class="btn btn-primary small"
            :disabled="kb.asking || !ai.status?.chat.configured || !ai.status?.embedding.configured"
            @click="askKb"
          >
            {{ kb.asking ? "回答中…" : "提问" }}
          </button>
        </div>
        <div v-if="kb.answer || kb.answerError" class="answer">
          <p v-if="kb.answerError" class="answer-error">{{ kb.answerError }}</p>
          <p v-else class="answer-text">{{ kb.answer }}</p>
          <div v-if="kb.answerSources.length" class="answer-sources">
            <span class="muted">引用：</span>
            <button
              v-for="s in kb.answerSources"
              :key="s.path"
              class="btn btn-ghost small"
              @click="openKbSourcePath(s.path)"
            >
              {{ baseName(s.path) }}
            </button>
          </div>
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped>
.archive {
  max-width: 640px;
}
h2 {
  font-size: 1rem;
  margin: 0 0 8px;
}
h3 {
  font-size: 0.85rem;
  margin: 22px 0 8px;
}
h3:first-child {
  margin-top: 0;
}
.intro {
  font-size: 0.85rem;
  line-height: 1.6;
  margin: 0 0 16px;
}
.hint {
  font-size: 0.78rem;
  line-height: 1.6;
  margin: 0 0 8px;
}
.list {
  padding: 4px 0;
}
.lrow,
.rrow,
.mrow,
.srow,
.krow {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 12px 16px;
}
.lrow + .lrow,
.rrow + .rrow,
.mrow + .mrow,
.srow + .srow,
.krow + .krow {
  border-top: 1px solid var(--aa-border);
}
.linfo,
.rinfo,
.minfo,
.sinfo,
.kinfo {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.lname,
.rname,
.mname,
.sname,
.kname {
  font-size: 0.88rem;
  font-weight: 600;
  word-break: break-all;
}
.mmeta,
.smeta,
.kmeta {
  font-size: 0.76rem;
}
.smeta.error {
  color: var(--aa-danger);
}
.kb-ask {
  padding: 16px;
  margin-top: 4px;
}
.ask-row {
  display: flex;
  gap: 8px;
}
.ask-row input[type="text"] {
  flex: 1;
  padding: 8px 12px;
  border: 1px solid var(--aa-border);
  border-radius: var(--aa-radius-sm);
  background: var(--aa-bg);
  color: var(--aa-text);
  font-size: 0.88rem;
}
.answer {
  margin-top: 12px;
  padding-top: 12px;
  border-top: 1px solid var(--aa-border);
}
.answer-text {
  font-size: 0.85rem;
  line-height: 1.6;
  white-space: pre-wrap;
}
.answer-error {
  font-size: 0.85rem;
  color: var(--aa-danger);
}
.answer-sources {
  margin-top: 10px;
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
  font-size: 0.8rem;
}
.cats {
  font-weight: 400;
  font-size: 0.78rem;
  margin-left: 6px;
}
.lmeta {
  font-size: 0.76rem;
}
.rtemplate,
.rtags {
  padding: 6px 10px;
  border: 1px solid var(--aa-border);
  border-radius: var(--aa-radius-sm);
  background: var(--aa-bg);
  color: var(--aa-text);
  font-size: 0.8rem;
}
.small {
  padding: 5px 12px;
  min-height: 32px;
  font-size: 0.8rem;
  flex-shrink: 0;
}
.empty {
  padding: 24px;
  text-align: center;
}
.banner {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 14px 16px;
  margin-bottom: 12px;
  font-size: 0.85rem;
}
.new-rule-toggle {
  margin-top: 10px;
}
.form {
  padding: 18px 20px;
  display: flex;
  flex-direction: column;
  gap: 14px;
  margin-top: 10px;
}
.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.field label {
  font-size: 0.85rem;
  font-weight: 600;
}
.field input[type="text"] {
  padding: 8px 12px;
  border: 1px solid var(--aa-border);
  border-radius: var(--aa-radius-sm);
  background: var(--aa-bg);
  color: var(--aa-text);
  font-size: 0.88rem;
}
.cat-checks {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
}
.cat-check {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: 0.8rem;
  font-weight: 400;
}
.actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

/* 开关（同 SettingsPage 既有样式） */
.switch {
  position: relative;
  width: 40px;
  height: 22px;
  flex-shrink: 0;
}
.switch input {
  opacity: 0;
  width: 0;
  height: 0;
}
.slider {
  position: absolute;
  inset: 0;
  background: var(--aa-border);
  border-radius: 22px;
  transition: background 0.15s;
}
.slider::before {
  content: "";
  position: absolute;
  height: 16px;
  width: 16px;
  left: 3px;
  top: 3px;
  background: #fff;
  border-radius: 50%;
  transition: transform 0.15s;
}
.switch input:checked + .slider {
  background: var(--aa-primary);
}
.switch input:checked + .slider::before {
  transform: translateX(18px);
}
</style>

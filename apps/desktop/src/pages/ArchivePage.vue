<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import { openPath } from "@tauri-apps/plugin-opener";
import { useArchiveStore } from "../stores/archive";
import { useToastStore } from "../stores/toast";
import { asCommandError } from "../lib/api";
import { timeText, baseName } from "../lib/format";
import type { ArchiveCategory, ArchiveRule } from "../lib/types";

const archive = useArchiveStore();
const toast = useToastStore();

onMounted(() => {
  void archive.loadAll();
});

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
</script>

<template>
  <div class="archive">
    <h2>归档</h2>
    <p class="intro muted">
      按规则自动把下载完成的文件分类归位（模型、图片、文档……）；AI 建议与本地知识库随后续版本上线。
    </p>

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
.rrow {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 12px 16px;
}
.lrow + .lrow,
.rrow + .rrow {
  border-top: 1px solid var(--aa-border);
}
.linfo,
.rinfo {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.lname,
.rname {
  font-size: 0.88rem;
  font-weight: 600;
  word-break: break-all;
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

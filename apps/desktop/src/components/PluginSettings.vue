<script setup lang="ts">
// 按插件自己声明的 schema 画设置表单（UI_DESIGN_SPEC §3.5 ③）。
//
// **这是设置页此前涨到 903 行的根治办法。** F3 之前，下载的 12 项与归档/AI 的 6 项
// 是在设置页里一个一个手写的 `<Field>` + `v-model` + 空值转换，占了整页的三分之二。
// 现在插件用 `Plugin::settings_schema()` 说清楚自己有哪些字段，这里一次性画完；
// 新插件、新字段都不需要动设置页。
//
// 「留空 = 不限」的数字输入尤其值得收拢：原来每个可空数字字段都要单独写一个
// computed 中转字符串，因为 `<input type="number">` 空值会被 Vue 强转成 `0`，
// 而 `0` 和「没设」在后端是完全不同的意思（0 是「限速为 0」）。写错一处就是个
// 很难发现的 bug，抄六遍更是。
import { computed } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { Field } from "./ui";

/** 插件设置字段的类型。`?` 后缀表示可空——可空的数字/字符串「留空」都要落成 `null`。 */
type FieldType =
  | "bool"
  | "u32"
  | "u32?"
  | "f64?"
  | "string?"
  | "text?"
  | "dir"
  | "file?";

interface SchemaField {
  key: string;
  type: FieldType;
  label: string;
  hint?: string;
}

interface Schema {
  title: string;
  /** 整个分区的提示，例如「改动需要重启应用才生效」。 */
  note?: string;
  fields: SchemaField[];
}

const props = defineProps<{
  schema: unknown;
  /** 这个插件当前的设置值（不透明 JSON）。 */
  modelValue: Record<string, unknown>;
}>();
const emit = defineEmits<{ "update:modelValue": [Record<string, unknown>] }>();

/** schema 是后端给的不透明 JSON——形状不对时**不画**，而不是画一个坏掉的表单。 */
const schema = computed<Schema | null>(() => {
  const s = props.schema as Schema | null;
  return s && Array.isArray(s.fields) ? s : null;
});

function set(key: string, value: unknown) {
  emit("update:modelValue", { ...props.modelValue, [key]: value });
}

/** 可空数字：`""` ⇄ `null`。见文件头注释——这正是原来每个字段抄一遍的那段。 */
function numberProxy(f: SchemaField) {
  return computed({
    get: () => {
      const v = props.modelValue[f.key];
      return v === null || v === undefined ? "" : String(v);
    },
    set: (v: string) => {
      const t = v.trim();
      if (t === "") {
        // 可空的落 null；不可空的（`u32`）回落成 0 会改变语义，所以留空时不动它。
        set(f.key, f.type.endsWith("?") ? null : props.modelValue[f.key]);
        return;
      }
      set(f.key, Number(t));
    },
  });
}

/** 可空字符串：`""` ⇄ `null`，避免存下一个全是空格的「已配置」假象。 */
function stringProxy(f: SchemaField) {
  return computed({
    get: () => (props.modelValue[f.key] as string | null) ?? "",
    set: (v: string) => set(f.key, v.trim() === "" ? null : v),
  });
}

async function pickDir(f: SchemaField) {
  const picked = await open({ directory: true, multiple: false });
  if (typeof picked === "string") set(f.key, picked);
}

async function pickFile(f: SchemaField) {
  const picked = await open({ directory: false, multiple: false });
  if (typeof picked === "string") set(f.key, picked);
}

function boolValue(f: SchemaField): boolean {
  return Boolean(props.modelValue[f.key]);
}
</script>

<template>
  <div v-if="schema" class="plugin-settings">
    <h3>{{ schema.title }}</h3>
    <p v-if="schema.note" class="note">{{ schema.note }}</p>

    <Field
      v-for="f in schema.fields"
      :key="f.key"
      :label="f.label"
      :hint="f.hint"
      :row="f.type === 'bool'"
    >
      <input
        v-if="f.type === 'bool'"
        type="checkbox"
        :checked="boolValue(f)"
        @change="set(f.key, ($event.target as HTMLInputElement).checked)"
      />

      <textarea
        v-else-if="f.type === 'text?'"
        :value="stringProxy(f).value"
        rows="3"
        @input="stringProxy(f).value = ($event.target as HTMLTextAreaElement).value"
      />

      <div v-else-if="f.type === 'dir' || f.type === 'file?'" class="path">
        <code>{{ modelValue[f.key] || "（未设置）" }}</code>
        <button
          class="pick"
          @click="f.type === 'dir' ? pickDir(f) : pickFile(f)"
        >
          选择…
        </button>
      </div>

      <input
        v-else-if="f.type === 'string?'"
        type="text"
        :value="stringProxy(f).value"
        @input="stringProxy(f).value = ($event.target as HTMLInputElement).value"
      />

      <!-- 剩下的都是数字（u32 / u32? / f64?）。用 type="text" 而不是 number：
           number 输入框空值会被强转成 0，而 0 与「没设」在后端含义完全不同。 -->
      <input
        v-else
        type="text"
        inputmode="decimal"
        :value="numberProxy(f).value"
        @input="numberProxy(f).value = ($event.target as HTMLInputElement).value"
      />
    </Field>
  </div>
</template>

<style scoped>
.plugin-settings {
  display: contents;
}
h3 {
  margin: var(--sp-5) 0 var(--sp-2);
  font-size: var(--fs-base);
  font-weight: var(--fw-bold);
}
.note {
  margin: 0 0 var(--sp-2);
  font-size: var(--fs-sm);
  color: var(--aa-warn);
  line-height: 1.5;
}
.path {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  min-width: 0;
}
.path code {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-sm);
  color: var(--aa-text-dim);
}
.pick {
  flex: none;
  padding: var(--sp-1) var(--sp-3);
  border: 1px solid var(--aa-border);
  border-radius: var(--aa-radius-sm);
  font-size: var(--fs-sm);
}
input[type="text"],
textarea {
  width: 100%;
  padding: var(--sp-2) var(--sp-3);
  border: 1px solid var(--aa-border);
  border-radius: var(--aa-radius-sm);
  background: var(--aa-bg);
  color: var(--aa-text);
  font: inherit;
  font-size: var(--fs-base);
}
textarea {
  resize: vertical;
}
</style>

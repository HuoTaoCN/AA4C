<script setup lang="ts">
// 设置页（UI_DESIGN_SPEC §3.5）。
//
// F3 之前：5 个标签页、28 个设置项、903 行——其中 18 项属于下载与归档/AI，
// 是这一页失控的直接原因。F1.3 把设置拆成「核心 10 项 + 插件自己的一格」之后，
// 这一页只需要画核心那 10 项，插件的部分交给 `PluginSettings` 按 schema 渲染。
//
// 已配对设备列表留在这里，但**只保留不可逆操作（解除配对）**——可逆的信任升降级
// 与确认引荐在设备页顺手完成。按可逆性分，不按「都放一起更整齐」分（§3.5）。
import { computed, onMounted, reactive, ref, watchEffect } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { useDeviceStore } from "../stores/devices";
import { usePluginStore } from "../stores/plugins";
import { useSettingsStore } from "../stores/settings";
import { useToastStore } from "../stores/toast";
import { api, asCommandError } from "../lib/api";
import { platformIcon } from "../lib/format";
import PluginSettings from "../components/PluginSettings.vue";
import { Button, Card, Field, Icon, Toolbar } from "../components/ui";
import type { LocalServerStatus, Settings } from "../lib/types";

const devices = useDeviceStore();
const plugins = usePluginStore();
const settings = useSettingsStore();
const toast = useToastStore();

// 本地编辑副本。一个 form 对应一次完整保存，所以只有一个「保存」按钮。
const form = reactive<Settings>({
  deviceName: "",
  saveDir: "",
  autoAcceptFromTrusted: false,
  listenPort: 42420,
  serverUrl: null,
  enableRemote: false,
  enablePortMapping: true,
  enableLocalServer: false,
  localServerPort: 42421,
  localServerHost: null,
  plugins: {},
});
watchEffect(() => {
  if (settings.settings) Object.assign(form, structuredClone(settings.settings));
});

/** 留空 ⇄ null，避免存下一个全是空格的「已配置」假象。 */
function nullableText(key: "serverUrl" | "localServerHost") {
  return computed({
    get: () => form[key] ?? "",
    set: (v: string) => {
      form[key] = v.trim() === "" ? null : v.trim();
    },
  });
}
const serverUrlInput = nullableText("serverUrl");
const localServerHostInput = nullableText("localServerHost");

const localServer = ref<LocalServerStatus | null>(null);
async function loadLocalServer() {
  try {
    localServer.value = await api.localServerStatus();
  } catch {
    localServer.value = null;
  }
}

/** 中转站对外是不是真的找得到。文案是后端给的三种情况的人话版本。 */
const reachNote = computed(() => {
  switch (localServer.value?.reach) {
    case "configured":
      return { warn: false, text: "用的是你填的地址，只要它一直指向这台设备就长期有效。" };
    case "detected":
      return {
        warn: true,
        text: "这是自动探测到的公网地址。现在能用，但家庭宽带的地址会变，变了这个地址就失效——填一个 DDNS 域名更稳。",
      };
    default:
      return {
        warn: true,
        text: "只找到局域网地址，出了这个局域网就连不上。要让外面的设备找到这台机器，需要填一个 DDNS 域名或固定地址。",
      };
  }
});

const portClash = computed(() => form.localServerPort === form.listenPort);

onMounted(() => {
  void loadLocalServer();
  void devices.loadDevices();
});

async function pickSaveDir() {
  const picked = await open({ directory: true, multiple: false });
  if (typeof picked === "string") form.saveDir = picked;
}

async function save() {
  try {
    await settings.save({ ...form });
    toast.push("success", "已保存");
    await loadLocalServer();
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}

async function copyServerAddress() {
  if (!localServer.value?.address) return;
  await writeText(localServer.value.address);
  toast.push("success", "地址已复制");
}

const paired = computed(() =>
  devices.devices.filter((d) => d.trusted && d.id !== devices.self?.id),
);

async function unpair(id: string, name: string) {
  try {
    await api.unpairDevice(id);
    await devices.loadDevices();
    toast.push("success", `已解除与「${name}」的配对`);
  } catch (e) {
    toast.push("error", asCommandError(e).message);
  }
}
</script>

<template>
  <div class="settings">
    <Toolbar title="设置">
      <template #actions>
        <Button variant="primary" @click="save">保存</Button>
      </template>
    </Toolbar>

    <!-- ① 这台设备 -->
    <section>
      <h2>这台设备</h2>
      <Card>
        <Field label="设备名称" hint="别人在「附近设备」里看到的名字">
          <input v-model="form.deviceName" type="text" maxlength="40" />
        </Field>

        <Field
          label="接收文件保存到"
          hint="收到的文件存这里。这个目录会自动纳入同步索引。"
        >
          <div class="path">
            <code>{{ form.saveDir || "（未设置）" }}</code>
            <Button size="sm" variant="ghost" @click="pickSaveDir">选择…</Button>
          </div>
        </Field>

        <Field
          label="我的设备来的文件自动接收"
          hint="打开后，标为「我的设备」的设备发来的文件不再逐次弹窗确认。"
          row
        >
          <input v-model="form.autoAcceptFromTrusted" type="checkbox" />
        </Field>
      </Card>
      <p class="lock"><Icon name="shield" :size="14" /> 所有传输均已加密</p>
    </section>

    <!-- ② 让不同网络的设备也能连上 -->
    <section>
      <h2>让不同网络的设备也能连上</h2>
      <Card>
        <Field
          label="中转站地址"
          hint="填自己搭的中转站地址后，不在同一局域网的已配对设备也能互相找到、经它收发文件。没有中转站不影响局域网内的正常使用。"
        >
          <input
            v-model="serverUrlInput"
            type="text"
            placeholder="aa4c://your-server:42420#指纹"
          />
        </Field>

        <Field
          label="开启远程连接"
          :hint="form.serverUrl ? undefined : '先填中转站地址才能打开这个开关。'"
          row
        >
          <input
            v-model="form.enableRemote"
            type="checkbox"
            :disabled="!form.serverUrl"
          />
        </Field>

        <Field
          label="自动在路由器上开端口"
          hint="让路由器把传输端口转发到这台设备，对方就能直接连过来，不必绕道中转站，速度更快。这会在你的路由器上开一个端口，退出时自动收回。要先打开「开启远程连接」才生效。"
          :tone="form.enablePortMapping && form.enableRemote ? 'warn' : 'muted'"
          row
        >
          <input
            v-model="form.enablePortMapping"
            type="checkbox"
            :disabled="!form.enableRemote"
          />
        </Field>
      </Card>

      <h2>让这台设备当中转站</h2>
      <Card>
        <Field
          label="在这台设备上运行中转站"
          hint="如果这台设备常年开着（台式机、NAS），可以让它兼任你自己的中转站，就不用另外租服务器了。但光打开这个开关，外面的设备还找不到它——你还需要一个稳定的入口地址，通常是给家里的宽带配一个 DDNS 域名。"
          row
        >
          <input v-model="form.enableLocalServer" type="checkbox" />
        </Field>

        <Field label="别人用什么地址找到这台设备">
          <input
            v-model="localServerHostInput"
            type="text"
            placeholder="例如 home.example.com（DDNS 域名）"
            :disabled="!form.enableLocalServer"
          />
        </Field>

        <Field
          label="中转站端口"
          :hint="portClash ? `不能和传输端口（${form.listenPort}）用同一个号，两边都要占这个端口。` : undefined"
          :tone="portClash ? 'warn' : 'muted'"
        >
          <input
            v-model.number="form.localServerPort"
            type="number"
            min="1"
            max="65535"
            :disabled="!form.enableLocalServer"
          />
        </Field>

        <Field
          v-if="localServer?.running && localServer.address"
          label="把这个地址填到你的其它设备上"
          :hint="reachNote.text"
          :tone="reachNote.warn ? 'warn' : 'muted'"
        >
          <div class="path">
            <code>{{ localServer.address }}</code>
            <Button size="sm" variant="ghost" @click="copyServerAddress">
              <Icon name="copy" :size="14" /> 复制
            </Button>
          </div>
        </Field>
        <p v-else-if="form.enableLocalServer" class="hint">
          中转站还没起来。保存设置后稍等一会儿；如果一直起不来，多半是这个端口已经被别的程序占了。
        </p>
      </Card>
    </section>

    <!-- ③ 插件设置：由插件自己声明的 schema 渲染，这一页不手写任何一项 -->
    <section v-if="plugins.installed.length">
      <h2>插件</h2>
      <Card>
        <PluginSettings
          v-for="p in plugins.installed"
          :key="p.id"
          :schema="p.settingsSchema"
          :model-value="(form.plugins[p.id] as Record<string, unknown>) ?? {}"
          @update:model-value="form.plugins[p.id] = $event"
        />
      </Card>
    </section>

    <!-- ④ 已配对设备：只放不可逆操作 -->
    <section>
      <h2>已配对设备</h2>
      <Card v-if="paired.length" padding="none">
        <div v-for="d in paired" :key="d.id" class="drow">
          <span class="ico">{{ platformIcon(d.platform) }}</span>
          <div class="dinfo">
            <div class="dname">{{ d.name }}</div>
            <div class="dsub">
              {{ d.trustLevel === "full" ? "我的设备" : "朋友" }}
            </div>
          </div>
          <Button size="sm" variant="danger" @click="unpair(d.id, d.name)">
            解除配对
          </Button>
        </div>
      </Card>
      <p v-else class="hint">还没有配对任何设备。</p>
      <p class="hint">
        升降「我的设备」与确认引荐来的设备在
        <router-link to="/">设备</router-link> 页——那些是可逆的，随手就能改；
        解除配对不可逆，所以放在这里。
      </p>
    </section>
  </div>
</template>

<style scoped>
.settings {
  max-width: 720px;
  display: flex;
  flex-direction: column;
  gap: var(--sp-5);
}
section {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
}
h2 {
  margin: 0;
  font-size: var(--fs-base);
  font-weight: var(--fw-bold);
  color: var(--aa-text-dim);
}
.hint {
  margin: 0;
  font-size: var(--fs-sm);
  color: var(--aa-text-dim);
  line-height: 1.5;
}
.lock {
  margin: 0;
  display: flex;
  align-items: center;
  gap: var(--sp-1);
  font-size: var(--fs-sm);
  color: var(--aa-text-dim);
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
.drow {
  display: flex;
  align-items: center;
  gap: var(--sp-3);
  padding: var(--sp-3) var(--sp-4);
}
.drow + .drow {
  border-top: 1px solid var(--aa-border);
}
.ico {
  font-size: var(--fs-xl);
  flex: none;
}
.dinfo {
  flex: 1;
  min-width: 0;
}
.dname {
  font-size: var(--fs-base);
  font-weight: var(--fw-medium);
}
.dsub {
  margin-top: var(--sp-1);
  font-size: var(--fs-sm);
  color: var(--aa-text-dim);
}
input[type="text"],
input[type="number"] {
  width: 100%;
  padding: var(--sp-2) var(--sp-3);
  border: 1px solid var(--aa-border);
  border-radius: var(--aa-radius-sm);
  background: var(--aa-bg);
  color: var(--aa-text);
  font: inherit;
  font-size: var(--fs-base);
}
</style>

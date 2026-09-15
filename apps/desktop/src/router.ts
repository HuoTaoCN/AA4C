import { createRouter, createWebHistory } from "vue-router";

// 路由与 `lib/nav.ts` 的导航配置一一对应（UI_DESIGN_SPEC §2）。
//
// V0.8「Focus」F3：`/` 从「首页宫格」变成**设备页**；`/records` 与 `/me` 消失——
// 记录并入传输页，「我的」那个杂物抽屉随移动端 5 tab 一起去掉了。
// `/download` 与 `/archive` 是插件页面，路由一直在（直接输地址也能到），
// 但导航项只在装了对应插件时出现。
export const router = createRouter({
  history: createWebHistory(),
  routes: [
    {
      path: "/",
      name: "devices",
      component: () => import("./pages/DevicesPage.vue"),
    },
    { path: "/send", name: "send", component: () => import("./pages/SendPage.vue") },
    { path: "/sync", name: "sync", component: () => import("./pages/SyncPage.vue") },
    {
      path: "/share",
      name: "share",
      component: () => import("./pages/SharePage.vue"),
    },
    {
      path: "/settings",
      name: "settings",
      component: () => import("./pages/SettingsPage.vue"),
    },

    // —— 插件页面（装了才在导航里出现） ——
    {
      path: "/download",
      name: "download",
      component: () => import("./pages/DownloadPage.vue"),
    },
    {
      path: "/archive",
      name: "archive",
      component: () => import("./pages/ArchivePage.vue"),
    },

    // 老路径重定向，免得有人存了书签或从旧版本升上来
    { path: "/records", redirect: "/send" },
    { path: "/me", redirect: "/" },
  ],
});

import { createRouter, createWebHistory } from "vue-router";

export const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: "/", name: "home", component: () => import("./pages/HomePage.vue") },
    { path: "/send", name: "send", component: () => import("./pages/SendPage.vue") },
    { path: "/sync", name: "sync", component: () => import("./pages/SyncPage.vue") },
    { path: "/share", name: "share", component: () => import("./pages/SharePage.vue") },
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
    {
      path: "/records",
      name: "records",
      component: () => import("./pages/RecordsPage.vue"),
    },
    {
      path: "/settings",
      name: "settings",
      component: () => import("./pages/SettingsPage.vue"),
    },
    { path: "/me", name: "me", component: () => import("./pages/MePage.vue") },
  ],
});

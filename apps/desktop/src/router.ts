import { createRouter, createWebHistory } from "vue-router";

export const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: "/", name: "home", component: () => import("./pages/HomePage.vue") },
    { path: "/send", name: "send", component: () => import("./pages/SendPage.vue") },
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
  ],
});

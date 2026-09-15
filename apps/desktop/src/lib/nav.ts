// 导航配置（UI_DESIGN_SPEC §2 信息架构）。
//
// V0.8「Focus」F3 把导航从 8 个目标砍到 5 个。F3 之前是
// 首页 / 传输 / 同步 / 分享 / 下载 / 归档 / 记录 / 设置，**而且首页本身是一个「能力」宫格，
// 格子里链的就是侧栏那五项**——菜单的菜单，是产品对「用户先做什么」没有意见的典型症状。
//
// 现在：设备（就是首页）/ 传输（含记录）/ 同步 / 分享 / 设置。
// 下载与归档是插件（F1），只在装了的时候出现在「更多」分区里——这正是本文件此前写着
// 却被违背了的那条原则：「不提前占位」。

import type { IconName } from "../components/ui";

export interface NavItem {
  path: string;
  /** 统一两个汉字。 */
  name: string;
  icon: IconName;
  /** 一句话说明（空状态引导、移动端「更多」列表用）。 */
  desc: string;
}

/** 主导航。顺序即侧栏顺序，也是移动端底部 tab 的顺序。 */
export const PRIMARY: NavItem[] = [
  {
    path: "/",
    name: "设备",
    icon: "devices",
    desc: "你的设备现在连上了吗、怎么连的",
  },
  {
    path: "/send",
    name: "传输",
    icon: "send",
    desc: "选文件 · 选设备 · AA，以及发过什么",
  },
  {
    path: "/sync",
    name: "同步",
    icon: "refresh",
    desc: "让文件夹在你的设备之间保持一致",
  },
  {
    path: "/share",
    name: "分享",
    icon: "link",
    desc: "生成一条链接，给还没配对的人",
  },
];

/** 设置单独放在侧栏底部——它是配置，不是能力。 */
export const SETTINGS: NavItem = {
  path: "/settings",
  name: "设置",
  icon: "settings",
  desc: "设备名、保存位置、让不同网络的设备也能连上",
};

/** 插件 id → 它的页面。**装了才出现**（见 `usePluginNav`）。
 *
 * 这张表只管「插件 id 对应哪个页面」，「装没装」由后端的 `plugin_manifest` 说了算——
 * 前端不再硬编码「有下载和归档这两个能力」。 */
export const PLUGIN_PAGES: Record<string, NavItem> = {
  download: {
    path: "/download",
    name: "下载",
    icon: "download",
    desc: "HTTP / FTP 直链与 BT / 磁力链接",
  },
  archive: {
    path: "/archive",
    name: "归档",
    icon: "folder-tree",
    desc: "按规则自动分类归档；AI 建议与本地知识库",
  },
};

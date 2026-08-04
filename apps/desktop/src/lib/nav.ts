// 导航与功能模块配置（整体 UI 架构，PROJECT_VISION §五/§六）。
//
// AA连接（AA4C）是跨平台设备连接平台：连接之上的五大能力（传输 / 同步 / 分享 / 下载 / 归档）。
// 五大能力至 V0.5 均已实现；仍未实现的模块（V0.6+）标记为建设中并注明计划版本。
// 记录 / 设置为次级入口（PC 侧栏底部分组、移动端「我的」聚合）。

export interface NavItem {
  path: string;
  /** 统一两个汉字。 */
  name: string;
  /** emoji 占位图标（与设备卡片风格一致，V0.2 换线性图标）。 */
  icon: string;
  /** 是否已实现；false 显示建设中。 */
  built: boolean;
  /** 计划版本（建设中模块）。 */
  version?: string;
  /** 一句话说明（首页能力卡片 / 建设中页用）。 */
  desc: string;
}

export const HOME: NavItem = {
  path: "/",
  name: "首页",
  icon: "🏠",
  built: true,
  desc: "设备、任务与最近文件总览",
};

/** 五大能力模块。 */
export const CAPABILITIES: NavItem[] = [
  {
    path: "/send",
    name: "传输",
    icon: "📤",
    built: true,
    desc: "选文件 · 选设备 · AA，局域网加密直传",
  },
  {
    path: "/sync",
    name: "同步",
    icon: "🔄",
    built: true,
    version: "V0.2",
    desc: "文件夹持续同步，支持单向 / 双向 / 增量",
  },
  {
    path: "/share",
    name: "分享",
    icon: "🔗",
    built: true,
    version: "V0.3",
    desc: "生成分享，把文件远程分享给好友、家庭、团队",
  },
  {
    path: "/download",
    name: "下载",
    icon: "⬇️",
    built: true,
    version: "V0.4",
    desc: "HTTP / HTTPS / FTP 直链下载，也支持 BT / 磁力链接",
  },
  {
    path: "/archive",
    name: "归档",
    icon: "🗂️",
    built: true,
    version: "V0.5",
    desc: "按规则自动分类、打标签、归档到指定目录；AI 建议辅助打标签，本地知识库可对自己的文件提问",
  },
];

/** 次级入口（不进主导航）。 */
export const UTILITY: NavItem[] = [
  { path: "/records", name: "记录", icon: "🕘", built: true, desc: "传输历史与重试" },
  {
    path: "/settings",
    name: "设置",
    icon: "⚙️",
    built: true,
    desc: "设备名、保存目录、已配对设备",
  },
];

/** 移动端底部标签（5 个，其余进「我的」）。 */
export const MOBILE_TABS: NavItem[] = [
  HOME,
  CAPABILITIES[0], // 传输
  CAPABILITIES[1], // 同步
  CAPABILITIES[3], // 下载
  { path: "/me", name: "我的", icon: "👤", built: true, desc: "记录、分享、归档、设置" },
];

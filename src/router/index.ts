import { createRouter, createWebHashHistory, type RouteRecordInfo, type RouteRecordRaw } from "vue-router";

/** 页面路由名集中定义，供导航入口和类型映射共用。 */
export const routeNames = {
  home: "home",
  shelf: "shelf",
  shelfImport: "shelf-import",
  search: "search",
  sources: "sources",
  bookDetail: "book-detail",
  settings: "settings",
  settingsAppearance: "settings-appearance",
  settingsBackup: "settings-backup",
  settingsHistory: "settings-history",
  settingsBookmarks: "settings-bookmarks",
  settingsTasks: "settings-tasks",
  settingsOther: "settings-other",
  settingsCache: "settings-cache",
  settingsNetwork: "settings-network",
  settingsTts: "settings-tts",
  settingsRules: "settings-rules",
  settingsHomeConfig: "settings-home-config",
  settingsTxtToc: "settings-txt-toc",
} as const;

export type AppRouteName = (typeof routeNames)[keyof typeof routeNames];
export type AppScreen = "home" | "shelf" | "search" | "sources" | "settings" | "detail";
export type SettingsSection = "home" | "appearance" | "backup" | "history" | "bookmarks" | "tasks" | "other" | "cache" | "network" | "tts" | "rules" | "home-config" | "txt-toc";

export const settingsRouteNames: Record<SettingsSection, AppRouteName> = {
  home: routeNames.settings,
  appearance: routeNames.settingsAppearance,
  backup: routeNames.settingsBackup,
  history: routeNames.settingsHistory,
  bookmarks: routeNames.settingsBookmarks,
  tasks: routeNames.settingsTasks,
  other: routeNames.settingsOther,
  cache: routeNames.settingsCache,
  network: routeNames.settingsNetwork,
  tts: routeNames.settingsTts,
  rules: routeNames.settingsRules,
  "home-config": routeNames.settingsHomeConfig,
  "txt-toc": routeNames.settingsTxtToc,
};

type EmptyRouteParams = Record<never, never>;
type StaticRoute<Name extends AppRouteName, Path extends string> = RouteRecordInfo<
  Name,
  Path,
  EmptyRouteParams,
  EmptyRouteParams
>;

/** 具名路由表让 router.push({ name }) 等调用获得路径和参数类型检查。 */
export interface AppRouteNamedMap {
  home: StaticRoute<typeof routeNames.home, "/">;
  shelf: StaticRoute<typeof routeNames.shelf, "/shelf">;
  "shelf-import": StaticRoute<typeof routeNames.shelfImport, "/shelf/import">;
  search: StaticRoute<typeof routeNames.search, "/search">;
  sources: StaticRoute<typeof routeNames.sources, "/sources">;
  "book-detail": StaticRoute<typeof routeNames.bookDetail, "/book-detail">;
  settings: StaticRoute<typeof routeNames.settings, "/settings">;
  "settings-appearance": StaticRoute<typeof routeNames.settingsAppearance, "/settings/appearance">;
  "settings-backup": StaticRoute<typeof routeNames.settingsBackup, "/settings/backup">;
  "settings-history": StaticRoute<typeof routeNames.settingsHistory, "/settings/history">;
  "settings-bookmarks": StaticRoute<typeof routeNames.settingsBookmarks, "/settings/bookmarks">;
  "settings-tasks": StaticRoute<typeof routeNames.settingsTasks, "/settings/tasks">;
  "settings-other": StaticRoute<typeof routeNames.settingsOther, "/settings/other">;
  "settings-cache": StaticRoute<typeof routeNames.settingsCache, "/settings/other/cache">;
  "settings-network": StaticRoute<typeof routeNames.settingsNetwork, "/settings/other/network">;
  "settings-tts": StaticRoute<typeof routeNames.settingsTts, "/settings/other/tts">;
  "settings-rules": StaticRoute<typeof routeNames.settingsRules, "/settings/rules">;
  "settings-home-config": StaticRoute<typeof routeNames.settingsHomeConfig, "/settings/home-config">;
  "settings-txt-toc": StaticRoute<typeof routeNames.settingsTxtToc, "/settings/txt-toc">;
}

declare module "vue-router" {
  interface TypesConfig {
    RouteNamedMap: AppRouteNamedMap;
  }

  interface RouteMeta {
    screen: AppScreen;
    settingsSection?: SettingsSection;
  }
}

const loadHomePage = () => import("../features/home/HomePage.vue");
const loadLibraryPage = () => import("../features/shelf/LibraryPage.vue");
const loadLibraryImportPage = () => import("../features/shelf/LibraryImportPage.vue");
const loadSourceWorkspace = () => import("../features/sources/SourceWorkspacePage.vue");
const loadSearchPage = () => import("../features/discovery/SearchPage.vue");
const loadBookDetailPage = () => import("../features/books/BookDetailPage.vue");
// 每个设置页面都是独立的懒加载路由，各自负责自己的页面内容。
const loadMePage = () => import("../features/settings/MePage.vue");
const loadAppearanceSettings = () => import("../features/settings/AppearanceSettingsPage.vue");
const loadBackupSettingsPage = () => import("../features/settings/BackupSettingsPage.vue");
const loadHomeConfigEditor = () => import("../features/home/HomeConfigEditor.vue");
const loadTxtTocRulesEditor = () => import("../features/settings/TxtTocRulesEditor.vue");
const loadOtherSettings = () => import("../features/settings/OtherSettingsPage.vue");
const loadChapterCacheSettings = () => import("../features/settings/ChapterCacheSettingsPage.vue");
const loadNetworkSettings = () => import("../features/settings/NetworkSettingsPage.vue");
const loadTtsSettings = () => import("../features/settings/TtsSettingsPage.vue");
const loadReplacementRules = () => import("../features/settings/ReplacementRulesPage.vue");
// 阅读记录与书签使用独立的完整页面。
const loadReadingHistoryPage = () => import("../features/settings/ReadingHistoryPage.vue");
const loadBookmarksPage = () => import("../features/settings/BookmarksPage.vue");
const loadTaskCenterPage = () => import("../features/tasks/TaskCenterPage.vue");

export const appRoutes = [
  { path: "/", name: routeNames.home, component: loadHomePage, meta: { screen: "home" } },
  { path: "/shelf", name: routeNames.shelf, component: loadLibraryPage, meta: { screen: "shelf" } },
  { path: "/shelf/import", name: routeNames.shelfImport, component: loadLibraryImportPage, meta: { screen: "shelf" } },
  { path: "/search", name: routeNames.search, component: loadSearchPage, meta: { screen: "search" } },
  { path: "/sources", name: routeNames.sources, component: loadSourceWorkspace, meta: { screen: "sources" } },
  { path: "/book-detail", name: routeNames.bookDetail, component: loadBookDetailPage, meta: { screen: "detail" } },
  { path: "/settings", name: routeNames.settings, component: loadMePage, meta: { screen: "settings", settingsSection: "home" } },
  {
    path: "/settings/appearance",
    name: routeNames.settingsAppearance,
    component: loadAppearanceSettings,
    meta: { screen: "settings", settingsSection: "appearance" },
  },
  { path: "/settings/backup", name: routeNames.settingsBackup, component: loadBackupSettingsPage, meta: { screen: "settings", settingsSection: "backup" } },
  {
    path: "/settings/history",
    name: routeNames.settingsHistory,
    component: loadReadingHistoryPage,
    meta: { screen: "settings", settingsSection: "history" },
  },
  {
    path: "/settings/bookmarks",
    name: routeNames.settingsBookmarks,
    component: loadBookmarksPage,
    meta: { screen: "settings", settingsSection: "bookmarks" },
  },
  {
    path: "/settings/tasks",
    name: routeNames.settingsTasks,
    component: loadTaskCenterPage,
    meta: { screen: "settings", settingsSection: "tasks" },
  },
  { path: "/settings/other", name: routeNames.settingsOther, component: loadOtherSettings, meta: { screen: "settings", settingsSection: "other" } },
  { path: "/settings/other/cache", name: routeNames.settingsCache, component: loadChapterCacheSettings, meta: { screen: "settings", settingsSection: "cache" } },
  { path: "/settings/other/network", name: routeNames.settingsNetwork, component: loadNetworkSettings, meta: { screen: "settings", settingsSection: "network" } },
  { path: "/settings/other/tts", name: routeNames.settingsTts, component: loadTtsSettings, meta: { screen: "settings", settingsSection: "tts" } },
  { path: "/settings/rules", name: routeNames.settingsRules, component: loadReplacementRules, meta: { screen: "settings", settingsSection: "rules" } },
  { path: "/settings/home-config", name: routeNames.settingsHomeConfig, component: loadHomeConfigEditor, meta: { screen: "settings", settingsSection: "home-config" } },
  { path: "/settings/txt-toc", name: routeNames.settingsTxtToc, component: loadTxtTocRulesEditor, meta: { screen: "settings", settingsSection: "txt-toc" } },
  { path: "/:pathMatch(.*)*", redirect: { name: routeNames.home } },
] satisfies RouteRecordRaw[];

const router = createRouter({
  history: createWebHashHistory(),
  routes: appRoutes,
});

export default router;

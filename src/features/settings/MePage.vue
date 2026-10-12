<script setup lang="ts">
import router, { routeNames, settingsRouteNames, type SettingsSection } from "../../router";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import PrimaryNavigation from "../../ui/PrimaryNavigation.vue";

type Item = {
  destination: SettingsSection | "sources";
  icon: "history" | "bookmark" | "download" | "source" | "appearance" | "backup" | "settings";
  title: string;
};

const contentItems: Item[] = [
  { destination: "history", icon: "history", title: "历史" },
  { destination: "bookmarks", icon: "bookmark", title: "书签" },
  { destination: "tasks", icon: "download", title: "下载与任务" },
  { destination: "sources", icon: "source", title: "内容引擎" },
];

const appItems: Item[] = [
  { destination: "appearance", icon: "appearance", title: "阅读与播放" },
  { destination: "backup", icon: "backup", title: "数据与同步" },
  { destination: "other", icon: "settings", title: "高级设置" },
];

function navigate(destination: Item["destination"]): void {
  void router.push({ name: destination === "sources" ? routeNames.sources : settingsRouteNames[destination] });
}
</script>

<template>
  <div class="me-page-layout">
    <PrimaryNavigation />
    <main class="me-page-main">
      <div class="ui-page-content settings-menu">
    <section v-for="group in ([{ title: '内容', items: contentItems }, { title: '应用', items: appItems }])"
      :key="group.title" class="settings-menu-section">
      <small class="settings-menu-label">{{ group.title }}</small>
      <button v-for="entry in group.items" :key="entry.destination" type="button" class="settings-menu-row"
        @click="navigate(entry.destination)">
        <span class="settings-menu-icon"><PrototypeIcon :name="entry.icon" /></span>
        <span class="settings-menu-copy">
          <strong>{{ entry.title }}</strong>
        </span>
        <PrototypeIcon name="right"/>
      </button>
    </section>
      </div>
    </main>
  </div>
</template>
<style scoped>
.me-page-layout {
  display: flex;
  width: 100%;
  height: 100dvh;
  min-height: 0;
  overflow: hidden;
  padding-top: var(--safe-top);
  background: var(--app-background);
}
.me-page-main {
  flex: 1;
  min-width: 0;
  min-height: 0;
  overflow-x: hidden;
  overflow-y: auto;
  padding: 0 0 var(--safe-bottom);
}
@media (max-width: 860px) {
  .me-page-main {
    padding: 0 0 calc(var(--ui-nav-height) + var(--safe-bottom));
  }
}
</style>

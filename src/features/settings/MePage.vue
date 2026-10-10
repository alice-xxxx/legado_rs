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
      <div class="me-prototype-list">
    <section v-for="group in ([{ title: '内容', items: contentItems }, { title: '应用', items: appItems }])"
      :key="group.title" class="me-prototype-section">
      <small class="me-prototype-section-label">{{ group.title }}</small>
      <button v-for="entry in group.items" :key="entry.destination" type="button" class="me-prototype-list-row"
        @click="navigate(entry.destination)">
        <span class="me-prototype-list-icon"><PrototypeIcon :name="entry.icon" /></span>
        <span class="me-prototype-list-copy">
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
  padding: 0 clamp(20px, 3.6vw, 56px) var(--safe-bottom);
}
.me-prototype-list {
  width: min(var(--app-content-width), calc(100% - 40px));
  margin: 0 auto;
  padding: 24px 0 54px;
}
.me-prototype-section {
  margin-bottom: 24px;
}
.me-prototype-section-label {
  display: block;
  margin: 0 4px 6px;
  color: var(--app-muted);
  font-size: 10px;
  font-weight: 650;
}
.me-prototype-list-row {
  display: grid;
  grid-template-columns: 40px minmax(0, 1fr) 20px;
  align-items: center;
  gap: 12px;
  width: 100%;
  min-height: 66px;
  padding: 11px 8px;
  border: 0;
  border-bottom: 1px solid var(--app-line);
  background: transparent;
  color: var(--app-text);
  text-align: left;
  cursor: pointer;
}
.me-prototype-list-row:hover {
  background: var(--app-accent-soft);
}
.me-prototype-list-icon {
  display: grid;
  place-items: center;
  width: 34px;
  height: 34px;
  border-radius: 10px;
  background: var(--app-icon-background);
  color: var(--app-icon-ink);
}
.me-prototype-list-icon :deep(svg),
.me-prototype-list-row > .prototype-icon {
  width: 18px;
  height: 18px;
}
.me-prototype-list-row > .prototype-icon {
  color: var(--app-subtle);
}
.me-prototype-list-copy {
  min-width: 0;
}
.me-prototype-list-copy strong {
  font-size: 12px;
  font-weight: 650;
}
@media (max-width: 860px) {
  .me-page-main {
    padding: 0 14px calc(67px + var(--safe-bottom));
  }
}
</style>

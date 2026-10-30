<script setup lang="ts">
import { List as KList } from "konsta/vue";
import router, { routeNames, settingsRouteNames, type SettingsSection } from "../../router";
import SettingsActionRow from "../../ui/SettingsActionRow.vue";

type Item = {
  destination: SettingsSection | "sources";
  icon: "history" | "bookmark" | "download" | "source" | "appearance" | "backup" | "settings" | "info";
  title: string;
};

const contentItems: Item[] = [
  { destination: "history", icon: "history", title: "历史" },
  { destination: "bookmarks", icon: "bookmark", title: "书签" },
  { destination: "sources", icon: "source", title: "内容引擎" },
];

const appItems: Item[] = [
  { destination: "appearance", icon: "appearance", title: "外观、阅读与播放" },
  { destination: "backup", icon: "backup", title: "数据与同步" },
  { destination: "other", icon: "settings", title: "高级设置" },
  { destination: "about", icon: "info", title: "关于" },
];

function navigate(destination: Item["destination"]): void {
  void router.push({ name: destination === "sources" ? routeNames.sources : settingsRouteNames[destination] });
}
</script>

<template>
  <div class="me-page-layout">
    <main class="me-page-main" aria-label="我的">
      <div class="ui-page-content">
        <section v-for="group in ([{ title: '内容', items: contentItems }, { title: '应用', items: appItems }])"
          :key="group.title" class="ui-settings-group">
          <h2>{{ group.title }}</h2>
          <KList strong inset dividers class="mx-0! my-0!">
            <SettingsActionRow v-for="entry in group.items" :key="entry.destination"
              :title="entry.title" :icon="entry.icon" @click="navigate(entry.destination)" />
          </KList>
        </section>
      </div>
    </main>
  </div>
</template>
<style scoped>
@layer components {
.me-page-layout {
  display: flex;
  width: 100%;
  height: 100dvh;
  min-height: 0;
  overflow: hidden;
  background: var(--app-background);
}
.me-page-main {
  flex: 1;
  min-width: 0;
  min-height: 0;
  overflow-x: hidden;
  overflow-y: auto;
  padding: var(--safe-top) 0 var(--safe-bottom);
}
@media (max-width: 860px) {
  .me-page-main {
    padding-bottom: calc(var(--ui-nav-height) + var(--safe-bottom));
  }
}
}
</style>

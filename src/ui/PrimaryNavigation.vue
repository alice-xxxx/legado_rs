<script setup lang="ts">
import { computed } from "vue";
import { useRoute } from "vue-router";
import { Button as KButton, Tabbar as KTabbar, TabbarLink as KTabbarLink, ToolbarPane as KToolbarPane, useTheme } from "konsta/vue";
import router, { routeNames } from "../router";
import PrototypeIcon from "./PrototypeIcon.vue";

type MainScreen = "home" | "shelf" | "settings";
const route = useRoute();
const theme = useTheme();
const activeScreen = computed(() => route.meta.screen);
const items = [
  { screen: "home", label: "发现", icon: "compass" },
  { screen: "shelf", label: "内容库", icon: "library" },
  { screen: "settings", label: "我的", icon: "user" },
] as const;
function navigate(screen: MainScreen): void {
  void router.replace({ name: routeNames[screen] });
}
</script>

<template>
  <aside class="side-rail">
    <nav class="rail-links" aria-label="主导航">
      <KButton v-for="item in items" :key="item.screen" type="button"
        class="h-16 w-full flex-col gap-1 px-0" rounded
        :tonal="activeScreen === item.screen" :clear="activeScreen !== item.screen"
        :aria-current="activeScreen === item.screen ? 'page' : undefined" @click="navigate(item.screen)">
        <PrototypeIcon :name="item.icon" />
        <span class="text-xs">{{ item.label }}</span>
      </KButton>
    </nav>
  </aside>
  <div class="mobile-nav">
    <KTabbar :key="theme" component="nav" icons labels aria-label="主导航">
      <KToolbarPane>
      <KTabbarLink v-for="item in items" :key="item.screen" component="button" type="button" role="button"
        :label="item.label" :active="activeScreen === item.screen"
        :aria-current="activeScreen === item.screen ? 'page' : undefined" @click="navigate(item.screen)">
        <template #icon><PrototypeIcon :name="item.icon" /></template>
      </KTabbarLink>
      </KToolbarPane>
    </KTabbar>
  </div>
</template>

<style scoped>
@layer components {
.side-rail { z-index: 20; display: flex; flex: 0 0 88px; align-items: center; width: 88px; padding: 12px 8px; border-right: 1px solid var(--app-line); background: var(--app-surface); }
.rail-links { display: grid; width: 100%; gap: 12px; }
.mobile-nav { display: none; }
.prototype-icon { width: 24px; height: 24px; }
@media (max-width: 860px) {
  .side-rail { display: none; }
  .mobile-nav { position: fixed; z-index: 45; inset: auto 0 0; display: block; }
}
}
</style>

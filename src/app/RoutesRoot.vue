<script setup lang="ts">
import { computed, KeepAlive, onBeforeUnmount, onMounted } from "vue";
import { RouterView, useRoute } from "vue-router";
import { routeNames } from "../router";
import PrimaryNavigation from "../ui/PrimaryNavigation.vue";
import { disposeSearchState } from "../features/discovery/searchState";
import { installNativeBackNavigation } from "./nativeBack";
import { installDialogInteractions } from "../ui/dialogInteractions";
import { initializeAppUpdates } from "../features/settings/appUpdates";

const route = useRoute();
const mainPage = computed(() => route.name === routeNames.home || route.name === routeNames.shelf || route.name === routeNames.settings);

onBeforeUnmount(disposeSearchState);
let disposeNativeBack: (() => void) | undefined;
let disposeDialogs: (() => void) | undefined;
let disposeUpdates: (() => void) | undefined;
onMounted(() => { disposeNativeBack = installNativeBackNavigation(); disposeDialogs = installDialogInteractions(); disposeUpdates = initializeAppUpdates(); });
onBeforeUnmount(() => disposeNativeBack?.());
onBeforeUnmount(() => disposeDialogs?.());
onBeforeUnmount(() => disposeUpdates?.());
</script>

<template>
  <div class="routes-layout">
    <PrimaryNavigation v-if="mainPage" />
    <div class="routes-page">
      <RouterView v-slot="{ Component }">
        <!-- 保留首页和搜索页；查看详情期间继续接收搜索进度，返回时复用已有状态。 -->
        <KeepAlive :include="['HomePage', 'SearchPage']">
          <component :is="Component" />
        </KeepAlive>
      </RouterView>
    </div>
  </div>
</template>

<style scoped>
@layer components {
.routes-layout { display: flex; width: 100%; height: 100dvh; min-width: 0; }
.routes-page { flex: 1; min-width: 0; min-height: 0; }
.routes-layout:has(.library-manage-page) > :deep(.side-rail),
.routes-layout:has(.library-manage-page) > :deep(.mobile-nav) { display: none; }
}
</style>

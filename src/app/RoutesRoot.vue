<script setup lang="ts">
import { KeepAlive, onBeforeUnmount, onMounted } from "vue";
import { RouterView } from "vue-router";
import { disposeSearchState } from "../features/discovery/searchState";
import { installNativeBackNavigation } from "./nativeBack";
import { installDialogInteractions } from "../ui/dialogInteractions";

onBeforeUnmount(disposeSearchState);
let disposeNativeBack: (() => void) | undefined;
let disposeDialogs: (() => void) | undefined;
onMounted(() => { disposeNativeBack = installNativeBackNavigation(); disposeDialogs = installDialogInteractions(); });
onBeforeUnmount(() => disposeNativeBack?.());
onBeforeUnmount(() => disposeDialogs?.());
</script>

<template>
  <RouterView v-slot="{ Component }">
    <!-- 保留首页和搜索页；查看详情期间继续接收搜索进度，返回时复用已有状态。 -->
    <KeepAlive :include="['HomePage', 'SearchPage']">
      <component :is="Component" />
    </KeepAlive>
  </RouterView>
</template>

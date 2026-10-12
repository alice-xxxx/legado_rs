<script setup lang="ts">
import { KeepAlive } from "vue";
import { RouterView } from "vue-router";
</script>

<template>
  <RouterView v-slot="{ Component }">
    <!-- 保留首页和搜索页；查看详情期间继续接收搜索进度，返回时复用已有状态。 -->
    <KeepAlive :include="['HomePage', 'SearchPage']">
      <component :is="Component" />
    </KeepAlive>
  </RouterView>
</template>

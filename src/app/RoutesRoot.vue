<script setup lang="ts">
import { KeepAlive } from "vue";
import { RouterView } from "vue-router";
</script>

<template>
  <RouterView v-slot="{ Component }">
    <!-- 只缓存首页；切换其它页面时保留首页内容，返回后不重复自动加载。 -->
    <KeepAlive include="HomePage">
      <component :is="Component" />
    </KeepAlive>
  </RouterView>
</template>

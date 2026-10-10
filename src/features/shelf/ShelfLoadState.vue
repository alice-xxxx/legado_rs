<script setup lang="ts">
defineProps<{
  loading: boolean;
  error: string;
  retry: () => void | Promise<void>;
}>();
</script>

<template>
  <section class="connection-state">
    <div class="connection-card">
      <div class="connection-symbol">{{ loading ? '…' : '!' }}</div>
      <h2>{{ loading ? '正在读取书架' : '暂时无法读取书架' }}</h2>
      <p>{{ loading ? '正在读取书架。' : error || '请检查应用状态后重试。' }}</p>
      <button class="button primary" :disabled="loading" @click="retry">{{ loading ? '加载中…' : '重新加载' }}</button>
    </div>
  </section>
</template>

<style scoped>
.connection-state {
  display: grid;
  min-height: 100%;
  place-items: center;
}

.connection-card {
  display: grid;
  max-width: 480px;
  justify-items: center;
  gap: 10px;
  padding: 24px;
  text-align: center;
}

.connection-card h2,
.connection-card p {
  margin: 0;
}

.connection-card p {
  color: var(--app-muted);
}
</style>

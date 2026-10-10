<script setup lang="ts">
// 二级页面共用同一套返回按钮，具体返回目标由页面路由决定。
import BackButton from "./BackButton.vue";

withDefaults(defineProps<{
  title?: string;
  backLabel?: string;
  showBackLabel?: boolean;
}>(), {
  title: "",
  backLabel: "返回",
  showBackLabel: false,
});

const emit = defineEmits<{ back: [] }>();
</script>

<template>
  <header class="subpage-header" :class="{ 'subpage-header--compact': !title }">
    <BackButton
      class="subpage-header__back"
      :label="backLabel"
      :show-label="showBackLabel"
      @click="emit('back')"
    />
    <div v-if="title" class="subpage-header__copy">
      <h2>{{ title }}</h2>
    </div>
    <div class="subpage-header__actions"><slot name="actions" /></div>
  </header>
</template>

<style scoped>
.subpage-header {
  position: sticky;
  top: 0;
  z-index: 30;
  display: flex;
  min-height: 64px;
  align-items: center;
  gap: 12px;
  padding: 0 16px;
  border-bottom: 1px solid var(--app-line);
  background: var(--app-background);
}

.subpage-header--compact {
  min-height: 48px;
}

.subpage-header__copy {
  min-width: 0;
  flex: 1;
}

.subpage-header__copy h2 {
  margin: 0;
  color: var(--app-text);
  font-size: 17px;
  font-weight: 700;
  line-height: 1.3;
}

.subpage-header__actions {
  display: flex;
  align-items: center;
  gap: 8px;
}

.subpage-header__back {
  color: var(--app-muted);
}

.subpage-header__back:hover:not(:disabled) {
  color: var(--app-accent);
}

</style>

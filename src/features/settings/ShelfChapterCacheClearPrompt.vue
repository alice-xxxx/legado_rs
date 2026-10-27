<script setup lang="ts">
import { Dialog as KDialog, DialogButton as KDialogButton } from "konsta/vue";
defineProps<{
  open: boolean;
  busy: boolean;
  locked: boolean;
  clear: () => void | Promise<void>;
  cancel: () => void;
}>();
</script>

<template>
  <section v-if="open || busy" class="modal-backdrop" @click.self="!busy && cancel()">
    <KDialog component="article" opened :backdrop="false" class="w-[min(560px,calc(100%-32px))] max-h-[88dvh] overflow-auto">
      <template #title><h2>{{ busy ? '正在清理在线章节缓存…' : '清理在线章节缓存？' }}</h2></template>
      <p v-if="busy">正在按书架顺序检查在线书籍并清理正文缓存，请保持应用打开。</p>
      <p v-else>将逐本清理当前书架中可更换源的在线书籍章节缓存。清理结果会显示成功、失败和未开始数量。</p>
      <p>本地 TXT、EPUB、CBZ、PDF 的章节资源、章节目录、阅读进度、封面和书签都会保留。若当前阅读器打开了本次要清理的在线书籍，会先保存阅读位置并关闭；位置未能保存时不会清理。</p>
      <template #buttons>
        <KDialogButton data-back-action :disabled="busy" @click="cancel">取消</KDialogButton>
        <KDialogButton strong class="k-color-brand-danger" :disabled="busy || locked" @click="clear">{{ busy ? '正在清理…' : '确认清理在线缓存' }}</KDialogButton>
      </template>
    </KDialog>
  </section>
</template>

<style scoped>
@layer components {
.modal-backdrop { position: fixed; z-index: 50; inset: 0; display: flex; align-items: center; justify-content: center; padding: 16px; background: var(--app-scrim); backdrop-filter: blur(3px); }
h2 { margin: 0; font: inherit; }
p { color: var(--app-muted); font-size: var(--ui-font-caption); line-height: 1.55; }
}
</style>

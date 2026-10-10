<script setup lang="ts">
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
    <article class="app-modal confirm-modal">
      <div class="confirm-symbol">⌫</div>
      <h2 id="chapter-cache-clear-title">{{ busy ? '正在清理在线章节缓存…' : '清理在线章节缓存？' }}</h2>
      <p v-if="busy">正在按书架顺序检查在线书籍并清理正文缓存，请保持应用打开。</p>
      <p v-else>将逐本清理当前书架中可更换书源的在线书籍章节缓存。清理结果会显示成功、失败和未开始数量。</p>
      <p>本地 TXT、EPUB、CBZ、PDF 的章节资源、章节目录、阅读进度、封面和书签都会保留。若当前阅读器打开了本次要清理的在线书籍，会先保存阅读位置并关闭；位置未能保存时不会清理。</p>
      <div class="modal-actions">
        <button class="button secondary" :disabled="busy" @click="cancel">取消</button>
        <button class="button danger" :disabled="busy || locked" @click="clear">{{ busy ? '正在清理…' : '确认清理在线缓存' }}</button>
      </div>
    </article>
  </section>
</template>

<style scoped>
.modal-backdrop { position: fixed; z-index: 50; inset: 0; display: flex; align-items: center; justify-content: center; padding: 16px; background: rgb(25 27 35 / 36%); backdrop-filter: blur(3px); }
.app-modal { width: min(560px, 100%); max-height: min(88dvh, 900px); overflow: auto; padding: 22px; border: 1px solid var(--app-line); border-radius: 16px; background: var(--app-surface); box-shadow: var(--app-shadow); }
.confirm-symbol { display: grid; width: 38px; height: 38px; place-items: center; margin-bottom: 12px; border-radius: 12px; color: var(--app-danger); background: #f8ebed; }
h2 { margin: 0 0 10px; font-size: 18px; }
p { color: var(--app-muted); font-size: 13px; line-height: 1.55; }
.modal-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 16px; }
</style>

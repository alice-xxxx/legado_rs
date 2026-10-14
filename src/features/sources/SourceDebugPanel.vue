<script setup lang="ts">
import type { ResourceDescriptor, SearchBookResult, SourceDebugResponse, SourceMetadata } from "../../api/types";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";

defineProps<{
  target: SourceMetadata;
  keyword: string;
  keywordLoading: boolean;
  busy: boolean;
  searchResultBatchBusy: boolean;
  recoveryRequired: boolean;
  stage: SourceDebugResponse["stage"] | null;
  error: string;
  searchErrors: Array<{ sourceId?: string; message: string }>;
  results: SearchBookResult[];
  searched: boolean;
  selectedResult: SearchBookResult | null;
  book: SourceDebugResponse["book"] | null;
  chapters: Array<{ index: number; title: string }>;
  selectedChapterIndex: number | null;
  chapterResource: ResourceDescriptor | null;
  chapterText: string;
}>();

const emit = defineEmits<{
  close: [];
  "update:keyword": [value: string];
  search: [];
  inspectResult: [result: SearchBookResult];
  previewChapter: [chapter: { index: number; title: string }];
}>();

function updateKeyword(event: Event): void {
  emit("update:keyword", (event.target as HTMLInputElement).value);
}

</script>

<template>
  <section class="source-debug-backdrop" data-back-layer @click.self="emit('close')" @keydown.esc.stop.prevent="emit('close')">
    <article class="source-debug-dialog" role="dialog" aria-modal="true" aria-labelledby="source-debug-title">
      <header class="source-debug-header">
        <h2 id="source-debug-title">测试来源</h2>
        <button type="button" class="source-debug-close" data-back-action aria-label="关闭测试" @click="emit('close')"><PrototypeIcon name="close" /></button>
      </header>
      <div class="source-debug-body">

      <form class="source-debug-search" @submit.prevent="emit('search')">
        <div class="source-debug-field">
          <span>来源</span>
          <span class="source-debug-source" :title="target.name">{{ target.name }}</span>
        </div>
        <label class="source-debug-field">
          <span>关键词</span>
          <input
            :value="keyword"
            autofocus
            maxlength="200"
            autocomplete="off"
            placeholder="输入测试关键词"
            :disabled="busy || keywordLoading"
            @input="updateKeyword"
          />
        </label>
        <div class="source-debug-actions">
          <button type="submit" class="button primary" :disabled="busy || keywordLoading || searchResultBatchBusy || recoveryRequired || !keyword.trim()">{{ busy ? '处理中…' : keywordLoading ? '读取中…' : '开始测试' }}</button>
        </div>
      </form>

      <p v-if="busy">正在处理{{ stage === 'result' ? '搜索' : stage === 'detail' ? '书籍详情' : stage === 'catalog' ? '章节目录' : stage === 'chapter' ? '章节正文' : '结果' }}…</p>
      <p v-if="error" class="error-message">{{ stage ? `阶段：${stage === 'result' ? '搜索结果' : stage === 'source' ? '源状态' : stage === 'detail' ? '详情' : stage === 'catalog' ? '目录' : '正文'}。` : '' }}{{ error }}</p>

      <div v-if="searchErrors.length" class="source-debug-errors">
        <strong>搜索阶段错误</strong>
        <p v-for="(issue, index) in searchErrors" :key="`${issue.sourceId || 'source'}-${index}`">{{ issue.message }}</p>
      </div>

      <section v-if="results.length" class="source-debug-results">
        <h3 class="source-debug-heading">处理后结果 · {{ results.length }}</h3>
        <article v-for="result in results" :key="result.resultId" class="source-debug-result">
          <div>
            <strong>{{ result.title }}</strong>
            <p class="source-debug-copy">{{ result.author || '作者未知' }}<span v-if="result.latestChapter"> · 最新：{{ result.latestChapter }}</span></p>
            <small v-if="result.intro">{{ result.intro }}</small>
          </div>
          <button type="button" class="button secondary small" :disabled="busy" @click="emit('inspectResult', result)">{{ selectedResult?.resultId === result.resultId ? '已选择' : '详情与目录' }}</button>
        </article>
      </section>
      <p v-if="searched && !results.length && !busy">没有找到匹配结果。</p>

      <section v-if="book" class="source-debug-book-detail">
        <h3 class="source-debug-heading">书籍详情 · {{ book.title }}</h3>
        <p class="source-debug-copy">{{ book.author || '作者未知' }}<span v-if="book.kind"> · {{ book.kind }}</span><span v-if="book.wordCount"> · {{ book.wordCount }}</span></p>
        <p v-if="book.intro" class="source-debug-book-intro">{{ book.intro }}</p>
        <strong>章节目录 · {{ chapters.length }}</strong>
        <div v-if="chapters.length" class="source-debug-chapters">
          <button v-for="chapter in chapters" :key="chapter.index" type="button" class="text-button source-debug-chapter-button" :disabled="busy" @click="emit('previewChapter', chapter)">{{ chapter.index + 1 }}. {{ chapter.title }}{{ selectedChapterIndex === chapter.index ? ' · 已选' : '' }}</button>
        </div>
        <p v-else>目录为空。</p>
      </section>

      <section v-if="chapterResource" class="source-debug-preview">
        <strong>章节正文预览</strong>
        <pre v-if="chapterResource.format === 'json'" class="source-debug-json">{{ chapterText }}</pre>
        <iframe v-else :key="chapterResource.resourceId" :src="chapterResource.src" title="源章节正文预览" sandbox="" class="source-debug-frame"></iframe>
      </section>
      </div>
    </article>
  </section>
</template>

<style scoped>
.source-debug-backdrop {
  position: fixed;
  inset: 0;
  z-index: 80;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 16px;
  background: rgb(25 27 35 / 36%);
}

.source-debug-dialog {
  display: flex;
  flex-direction: column;
  width: min(640px, 100%);
  max-height: min(88dvh, 840px);
  min-width: 0;
  overflow: hidden;
  border: 1px solid var(--app-line);
  border-radius: 12px;
  background: var(--app-surface);
  box-shadow: var(--app-shadow);
}

.source-debug-header {
  display: flex;
  flex: none;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 20px;
  border-bottom: 1px solid var(--app-line);
}

.source-debug-header h2 {
  margin: 0;
  font-size: var(--ui-font-heading);
}

.source-debug-close {
  display: grid;
  place-items: center;
  flex: none;
  width: 36px;
  height: 36px;
  border-radius: var(--ui-radius);
  color: var(--app-muted);
  cursor: pointer;
}

.source-debug-close :deep(svg) {
  width: var(--ui-icon-size);
  height: var(--ui-icon-size);
}

.source-debug-body {
  min-width: 0;
  min-height: 0;
  padding: 16px 20px 20px;
  overflow-y: auto;
  overflow-x: hidden;
  overflow-wrap: anywhere;
  overscroll-behavior: contain;
  font-size: var(--ui-font-body);
}

.source-debug-field {
  display: grid;
  grid-template-columns: 56px minmax(0, 1fr);
  align-items: center;
  gap: 12px;
}

.source-debug-field input {
  width: 100%;
  min-width: 0;
  height: var(--ui-control-height);
  padding: 0 10px;
  border: 1px solid var(--app-line);
  border-radius: var(--ui-radius);
  color: var(--app-text);
  background: var(--app-surface);
  font-size: var(--ui-font-body);
}

.source-debug-source {
  min-width: 0;
  overflow-wrap: anywhere;
}

.source-debug-actions {
  display: flex;
  justify-content: flex-end;
  padding-top: 4px;
}

.source-debug-actions .button {
  min-height: var(--ui-control-height);
  font-weight: 500;
}

.source-debug-close:focus-visible,
.source-debug-field input:focus-visible {
  outline: 2px solid var(--app-accent);
  outline-offset: 2px;
}

.source-debug-search,
.source-debug-results,
.source-debug-book-detail {
  min-width: 0;
  display: grid;
  gap: 12px;
}

.source-debug-errors {
  display: grid;
  gap: 6px;
  margin: 12px 0;
  color: var(--app-danger);
  min-width: 0;
  overflow-wrap: anywhere;
}

.source-debug-errors p,
.source-debug-copy {
  margin: 0;
}

.source-debug-results,
.source-debug-book-detail,
.source-debug-preview {
  margin-top: 16px;
}

.source-debug-heading {
  margin: 0;
  font-size: var(--ui-font-body);
}

.source-debug-result {
  min-width: 0;
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: center;
  gap: 8px;
  padding: 10px;
  border: 1px solid var(--app-line);
  border-radius: 10px;
}

.source-debug-result > div {
  min-width: 0;
}

.source-debug-book-detail {
  padding-top: 14px;
  border-top: 1px solid var(--app-line);
}

.source-debug-book-intro {
  white-space: pre-wrap;
}

.source-debug-chapters {
  display: grid;
  max-height: 230px;
  gap: 4px;
  overflow-y: auto;
  overflow-x: hidden;
}

.source-debug-chapter-button {
  text-align: left;
}

.source-debug-json,
.source-debug-frame {
  display: block;
  width: 100%;
  margin-top: 8px;
  border: 1px solid var(--app-line);
  border-radius: 8px;
  background: var(--app-surface);
}

.source-debug-json {
  box-sizing: border-box;
  max-height: 320px;
  overflow-y: auto;
  overflow-x: hidden;
  padding: 12px;
  white-space: pre-wrap;
}

.source-debug-frame {
  height: 320px;
}
</style>

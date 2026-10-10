<script setup lang="ts">
import type { ResourceDescriptor, SearchBookResult, SourceDebugResponse, SourceMetadata } from "../../api/types";

defineProps<{
  target: SourceMetadata;
  sources: SourceMetadata[];
  keyword: string;
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
  selectSource: [sourceId: string];
  search: [];
  inspectResult: [result: SearchBookResult];
  previewChapter: [chapter: { index: number; title: string }];
}>();

function updateKeyword(event: Event): void {
  emit("update:keyword", (event.target as HTMLInputElement).value);
}

function selectSource(event: Event): void {
  emit("selectSource", (event.target as HTMLSelectElement).value);
}
</script>

<template>
  <section class="modal-backdrop" @click.self="emit('close')">
    <article class="app-modal source-edit-modal source-debug-dialog">
      <button class="detail-close" @click="emit('close')">×</button>
      <h2 id="source-debug-title">测试「{{ target.name }}」</h2>

      <form class="source-debug-search" @submit.prevent="emit('search')">
        <label class="form-field">
          <span>书源</span>
          <select :value="target.id" :disabled="busy" @change="selectSource">
            <option v-for="source in sources" :key="source.id" :value="source.id">{{ source.name }}</option>
          </select>
        </label>
        <label class="form-field">
          <span>测试关键词</span>
          <input
            :value="keyword"
            autofocus
            maxlength="200"
            autocomplete="off"
            placeholder="输入书名、作者或关键词"
            :disabled="busy"
            @input="updateKeyword"
          />
        </label>
        <div class="modal-actions">
          <button type="button" class="button secondary" @click="emit('close')">关闭</button>
          <button type="submit" class="button primary" :disabled="busy || searchResultBatchBusy || recoveryRequired || !keyword.trim()">{{ busy ? '正在处理…' : '开始单源搜索' }}</button>
        </div>
      </form>

      <p v-if="busy">正在处理{{ stage === 'result' ? '搜索' : stage === 'detail' ? '书籍详情' : stage === 'catalog' ? '章节目录' : stage === 'chapter' ? '章节正文' : '结果' }}…</p>
      <p v-if="error" class="error-message">{{ stage ? `阶段：${stage === 'result' ? '搜索结果' : stage === 'source' ? '书源状态' : stage === 'detail' ? '详情' : stage === 'catalog' ? '目录' : '正文'}。` : '' }}{{ error }}</p>

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
        <iframe v-else :key="chapterResource.resourceId" :src="chapterResource.src" title="书源章节正文预览" sandbox="" class="source-debug-frame"></iframe>
      </section>
    </article>
  </section>
</template>

<style scoped>
.source-debug-dialog {
  position: relative;
  width: min(760px, 100%);
}

.source-debug-dialog h2 {
  margin: 0 0 10px;
}

.source-debug-search,
.source-debug-results,
.source-debug-book-detail {
  display: grid;
  gap: 12px;
}

.source-debug-errors {
  display: grid;
  gap: 6px;
  margin: 12px 0;
  color: #a34f46;
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
}

.source-debug-result {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: center;
  gap: 8px;
  padding: 10px;
  border: 1px solid var(--app-line);
  border-radius: 10px;
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
  overflow: auto;
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
  overflow: auto;
  padding: 12px;
  white-space: pre-wrap;
}

.source-debug-frame {
  height: 320px;
}
</style>

<script setup lang="ts">
import { computed } from "vue";
import type { BookResource, SearchResultSource } from "../../api/types";
import type { BookSourceSwitchSession } from "./bookSourceSwitchSession";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";

const props = defineProps<{
  book: BookResource | SearchResultSource;
  candidates: SearchResultSource[];
  session: BookSourceSwitchSession;
  loading?: boolean;
  locked?: boolean;
  error?: string;
  canRefresh?: boolean;
  refreshBusy?: boolean;
  canCheck?: boolean;
  checkBusy?: boolean;
}>();
const emit = defineEmits<{ select: [candidate: SearchResultSource]; search: []; refresh: []; check: []; retry: [] }>();
const active = computed(() => props.session.bookSourceSwitchOpen && "id" in props.book && props.session.bookSourceSwitchBook?.id === props.book.id);
const searching = computed(() => active.value && props.session.bookSourceSearchBusy);
const canSearch = computed(() => "canChangeSource" in props.book && props.book.canChangeSource && props.session.bookSourceEligibleSources.length > 0);
const busy = computed(() => props.locked || props.session.bookSourceSwitchBusy);
const message = computed(() => props.error || (active.value ? props.session.bookSourceSwitchError || props.session.bookSourceChangeError || props.session.chapterSourceError : ""));
const canReplaceChapter = computed(() => active.value && props.session.bookSourceCurrentChapter && !props.session.bookSourceSwitchBook?.mediaType);
function hideBrokenCover(event: Event): void { (event.target as HTMLImageElement).hidden = true; }
</script>

<template>
  <section class="book-sources-panel">
    <section class="source-section">
      <h2>当前来源</h2>
      <article class="source-current">
        <span class="source-cover" aria-hidden="true"><span>{{ book.title.slice(0, 1) }}</span><img v-if="book.coverSrc" :src="book.coverSrc" decoding="async" alt="" @error="hideBrokenCover" /></span>
        <div class="source-copy">
          <strong>{{ book.sourceName || '本地内容' }}</strong>
          <span>{{ book.title }}<template v-if="book.author"> · {{ book.author }}</template></span>
          <small v-if="book.latestChapter">最新：{{ book.latestChapter }}</small>
        </div>
        <button v-if="canRefresh" type="button" :disabled="refreshBusy || busy" @click="emit('refresh')">{{ refreshBusy ? '刷新中…' : '刷新信息' }}</button>
      </article>
    </section>
    <section class="source-section">
      <header class="source-heading">
        <h2>可更换源</h2>
        <button v-if="canSearch" type="button" :disabled="searching || busy" @click="emit('search')"><PrototypeIcon name="search" /><span>{{ searching ? '正在搜索…' : '搜索更多' }}</span></button>
      </header>
      <p v-if="loading" class="source-notice" role="status">正在加载详情和目录，可直接选择其他来源。</p>
      <p v-if="searching" class="source-notice" role="status">{{ session.bookSourceSearchStatus }} <button type="button" class="text-button" @click="session.stopBookSourceCandidateSearch">停止搜索</button></p>
      <p v-if="active && session.bookSourceChapterTargetId" class="source-notice">所选章节：{{ session.bookSourceChapterTargetTitle }}</p>
      <p v-if="message" class="source-error">{{ message }} <button v-if="error" type="button" class="text-button" @click="emit('retry')">重试</button></p>
      <div class="source-candidates">
        <article v-for="candidate in candidates" :key="candidate.resultId" class="source-candidate" :data-result-id="candidate.resultId">
          <span class="source-cover" aria-hidden="true"><span>{{ candidate.title.slice(0, 1) }}</span><img v-if="candidate.coverSrc" :src="candidate.coverSrc" loading="lazy" decoding="async" alt="" @error="hideBrokenCover" /></span>
          <div class="source-copy">
            <strong>{{ candidate.sourceName }}</strong>
            <span>{{ candidate.title }} · {{ candidate.author || '作者未提供' }}</span>
            <small>{{ candidate.latestChapter ? `最新：${candidate.latestChapter}` : '最新章节暂未提供' }}</small>
            <small v-if="candidate.requiresIdentityConfirmation" class="source-warning">作者未验证，更换前需要确认</small>
          </div>
          <span v-if="candidate.sourceId === book.sourceId" class="source-current-label">{{ loading ? '加载中…' : '当前' }}</span>
          <div v-else class="source-actions">
            <button type="button" :disabled="busy" @click="emit('select', candidate)">{{ session.bookSourceSwitchBusy && session.selectedBookSourceCandidate?.resultId === candidate.resultId ? '正在换源…' : '换源' }}</button>
            <button v-if="canReplaceChapter" type="button" :disabled="busy" @click="session.replaceCurrentChapterFromCandidate(candidate, session.chapterSourceIdentityConfirmResultId === candidate.resultId)">{{ session.chapterSourceBusyResultId === candidate.resultId ? '替换中…' : session.chapterSourceIdentityConfirmResultId === candidate.resultId ? '确认替换' : '替换本章' }}</button>
          </div>
        </article>
      </div>
      <p v-if="!candidates.length" class="source-empty">{{ searching ? '正在查找其他来源，找到后会显示在这里。' : loading ? '其他来源的搜索结果到达后会显示在这里。' : '暂无其他来源，可点击「搜索更多」查找。' }}</p>
      <p v-if="active && session.bookSourceCandidateErrors.length" class="source-partial">部分来源搜索失败（{{ session.bookSourceCandidateErrors.length }} 个），已找到的来源仍可选择。</p>
      <button v-if="canCheck" type="button" class="source-check" :disabled="checkBusy || busy" @click="emit('check')">{{ checkBusy ? '检查更新中…' : '检查内容更新' }}</button>
    </section>
  </section>
</template>

<style scoped>
.book-sources-panel { width: 100%; max-width: 850px; margin: 0 auto; color: var(--app-text); }
.source-section + .source-section { margin-top: 22px; padding-top: 18px; border-top: 1px solid var(--app-line); }
h2 { margin: 0 0 12px; font-size: var(--ui-font-body); }
.source-heading { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 12px; }
.source-heading h2 { margin: 0; }
.source-current, .source-candidate { display: grid; min-width: 0; grid-template-columns: 46px minmax(0, 1fr) auto; align-items: center; gap: 12px; }
.source-current { padding: 14px; border: 1px solid var(--app-line); border-radius: 14px; background: var(--app-surface); }
.source-cover { display: grid; position: relative; width: 46px; height: 64px; overflow: hidden; align-items: center; justify-items: center; border-radius: 6px; color: var(--app-accent-ink); background: var(--app-accent-soft); font-size: 20px; }
.source-cover img { position: absolute; width: 100%; height: 100%; object-fit: cover; }
.source-cover img[hidden] { display: none; }
.source-copy { display: grid; min-width: 0; gap: 4px; }
.source-copy strong { font-size: var(--ui-font-body); line-height: 1.45; overflow-wrap: anywhere; }
.source-copy span, .source-copy small { overflow: hidden; color: var(--app-muted); font-size: var(--ui-font-caption); line-height: 1.5; text-overflow: ellipsis; white-space: nowrap; }
.source-candidates { border-top: 1px solid var(--app-line); }
.source-candidate { min-height: 88px; padding: 12px 0; border-bottom: 1px solid var(--app-line); }
.source-actions { display: grid; justify-items: stretch; gap: 7px; }
button { display: inline-flex; min-height: 32px; align-items: center; justify-content: center; gap: 5px; padding: 0 10px; border: 1px solid var(--app-line); border-radius: 8px; color: var(--app-accent-ink); background: var(--app-surface); font-size: var(--ui-font-caption); white-space: nowrap; cursor: pointer; }
.source-actions button { border: 0; background: var(--app-panel-soft); }
.source-current-label { color: var(--app-success); font-size: var(--ui-font-caption); }
.source-notice { margin: 12px 0; padding: 12px; border-radius: 12px; color: var(--app-accent-ink); background: var(--app-accent-soft); font-size: var(--ui-font-caption); line-height: 1.7; }
.source-notice button, .source-error button { min-height: 28px; margin-left: 6px; padding: 0 4px; border: 0; background: transparent; }
.source-empty, .source-partial { padding: 12px 0; color: var(--app-muted); font-size: var(--ui-font-caption); line-height: 1.6; }
.source-error { color: var(--app-danger); font-size: var(--ui-font-caption); line-height: 1.6; overflow-wrap: anywhere; }
.source-copy .source-warning { color: var(--app-warning); white-space: normal; }
.source-check { margin-top: 14px; }
button:disabled { opacity: .5; cursor: default; }
button:focus-visible { outline: 2px solid var(--app-accent); outline-offset: 2px; }
@media (max-width: 430px) { .source-current, .source-candidate { gap: 9px; } .source-current { padding: 10px; } }
</style>

<script setup lang="ts">
import { Button as KButton } from "konsta/vue";
import { computed } from "vue";
import type { BookResource, SearchResultSource } from "../../api/types";
import type { BookSourceSwitchSession } from "./bookSourceSwitchSession";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import { compareSearchSources } from "../discovery/searchPresentation";

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
const busy = computed(() => props.locked || props.checkBusy || props.refreshBusy || props.session.bookSourceSwitchBusy || props.session.bookSourceCheckBusy);
const message = computed(() => props.error || (active.value ? props.session.bookSourceSwitchError || props.session.bookSourceChangeError : ""));
const sourceCandidates = computed<SearchResultSource[]>(() => {
  const book = props.book;
  return [{
    resultId: "resultId" in book ? book.resultId : `current-${book.id}`,
    sourceId: book.sourceId ?? "", sourceName: book.sourceName || "本地内容", bookUrl: book.bookUrl ?? "",
    title: book.title, author: book.author, coverSrc: book.coverSrc, latestChapter: book.latestChapter,
    responseTimeMs: "responseTimeMs" in book && typeof book.responseTimeMs === "number" ? book.responseTimeMs : undefined,
  }, ...props.candidates.filter(candidate => candidate.sourceId !== book.sourceId).sort(compareSearchSources)];
});
</script>

<template>
  <section class="book-sources-panel">
    <section class="source-section">
      <header class="source-heading">
        <h2>可更换源</h2>
        <div class="source-heading-actions">
          <KButton rounded inline clear class="gap-2" v-if="canCheck" type="button" :disabled="checkBusy || busy" @click="emit('check')">{{ checkBusy ? '检查中…' : '检查更新' }}</KButton>
          <KButton rounded inline clear class="gap-2" v-if="canSearch" type="button" :disabled="searching || busy" @click="emit('search')"><PrototypeIcon name="search" /><span>{{ searching ? '正在搜索…' : '搜索更多' }}</span></KButton>
        </div>
      </header>
      <p v-if="loading" class="status-notice" role="status">正在加载详情和目录，可直接选择其他来源。</p>
      <p v-if="searching" class="status-notice" role="status">{{ session.bookSourceSearchStatus }} <KButton rounded inline clear type="button" class="gap-2" @click="session.stopBookSourceCandidateSearch">停止搜索</KButton></p>
      <p v-if="session.bookSourceSwitchCancellable" class="status-notice" role="status">
        正在获取新来源… <KButton rounded inline clear type="button" class="gap-2" @click="session.closeBookSourceSwitch">取消换源</KButton>
      </p>
      <p v-if="message" class="status-notice is-error">{{ message }} <KButton rounded inline clear v-if="error" type="button" class="gap-2" @click="emit('retry')">重试</KButton></p>
      <div class="source-candidates">
        <article v-for="candidate in sourceCandidates" :key="candidate.resultId" class="source-candidate" :data-result-id="candidate.resultId">
          <span class="source-cover" aria-hidden="true">
            <span>{{ candidate.title.slice(0, 1) }}</span>
            <img v-if="candidate.coverSrc" :src="candidate.coverSrc" loading="lazy" decoding="async" alt=""
              @error="($event.target as HTMLImageElement).hidden = true" @load="($event.target as HTMLImageElement).hidden = false" />
          </span>
          <div class="source-copy">
            <div class="source-name-line">
              <strong>{{ candidate.sourceName }}</strong>
              <span v-if="candidate.sourceId === (book.sourceId ?? '')" class="source-current-label">当前</span>
            </div>
            <span class="source-book">{{ candidate.title }} · {{ candidate.author || '作者未知' }}</span>
            <div v-if="candidate.latestChapter || candidate.responseTimeMs != null" class="source-meta">
              <small v-if="candidate.latestChapter" class="source-latest" :title="candidate.latestChapter">{{ candidate.latestChapter }}</small>
              <small v-if="candidate.responseTimeMs != null" class="source-latency" :title="`响应耗时 ${candidate.responseTimeMs} 毫秒`">{{ candidate.responseTimeMs }} ms</small>
            </div>
          </div>
          <div v-if="candidate.sourceId === (book.sourceId ?? '')" class="source-actions">
            <KButton rounded inline clear class="gap-2" v-if="canRefresh" type="button" :disabled="refreshBusy || busy" @click="emit('refresh')">{{ refreshBusy ? '刷新中…' : '刷新信息' }}</KButton>
          </div>
          <div v-else class="source-actions">
            <KButton rounded inline clear class="gap-2" type="button" :disabled="busy" @click="emit('select', candidate)">{{ session.bookSourceSwitchBusy && session.selectedBookSourceCandidate?.resultId === candidate.resultId ? '正在换源…' : '换源' }}</KButton>
          </div>
        </article>
      </div>
      <div v-if="sourceCandidates.length === 1" class="page-empty-state">暂无其他来源。</div>
    </section>
  </section>
</template>

<style scoped>
@layer components {
.book-sources-panel { width: 100%; margin-inline: auto; color: var(--app-text); }
.source-heading { display: flex; align-items: center; justify-content: space-between; gap: 8px; margin-bottom: 8px; }
.source-heading h2 { flex: none; margin: 0; color: var(--app-muted); font-size: var(--ui-font-caption); font-weight: 650; }
.source-heading-actions { display: flex; align-items: center; gap: 6px; }
button { display: inline-flex; align-items: center; justify-content: center; gap: 5px; min-height: var(--ui-compact-control-height); padding: 0 10px; border: 1px solid var(--app-line); border-radius: var(--ui-radius); color: var(--app-accent-ink); background: var(--app-surface); font-size: var(--ui-font-caption); white-space: nowrap; cursor: pointer; }
.source-heading-actions button { min-height: 28px; padding: 0 4px; border: 0; color: var(--app-muted); background: transparent; }
button .prototype-icon { width: 16px; height: 16px; }
.source-candidates { border-top: 1px solid var(--app-line); }
.source-candidate { display: grid; grid-template-columns: 46px minmax(0, 1fr) auto; align-items: center; gap: 12px; min-height: 88px; padding: 12px 0; border-bottom: 1px solid var(--app-line); }
.source-cover { display: grid; position: relative; place-items: center; width: 46px; height: 64px; overflow: hidden; border-radius: var(--ui-radius); color: var(--app-icon-ink); background: var(--app-icon-background); font-size: var(--ui-font-heading); }
.source-cover img { position: absolute; width: 100%; height: 100%; object-fit: cover; }
.source-cover img[hidden] { display: none; }
.source-copy { display: grid; gap: 4px; min-width: 0; }
.source-book { overflow: hidden; color: var(--app-muted); font-size: var(--ui-font-caption); text-overflow: ellipsis; white-space: nowrap; }
.source-name-line { display: flex; align-items: center; gap: 8px; min-width: 0; }
.source-name-line strong { min-width: 0; font-size: var(--ui-font-body); line-height: 1.5; overflow-wrap: anywhere; }
.source-current-label { flex: none; color: var(--app-success); font-size: var(--ui-font-caption); }
.source-meta { display: flex; align-items: baseline; flex-wrap: wrap; gap: 4px 12px; min-width: 0; }
.source-meta small { color: var(--app-muted); font-size: var(--ui-font-caption); line-height: 1.5; }
.source-latest { min-width: 0; max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.source-latency { flex: none; white-space: nowrap; font-variant-numeric: tabular-nums; }
.source-actions { display: flex; }
.source-actions button { min-width: 64px; border-color: transparent; background: var(--app-panel-soft); }
}
</style>

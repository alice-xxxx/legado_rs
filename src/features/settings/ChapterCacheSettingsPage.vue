<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useRoute } from "vue-router";
import { errorText, notify } from "../../app/notifications";
import {
  getFrontendRestoreRevision,
  shelfBatchRecoveryRequired,
} from "../../app/recoveryState";
import { getReaderSession } from "../reader/readerSession";
import { backupBusy } from "./backupState";
import { useChapterCacheActions } from "./useChapterCacheActions";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import ShelfChapterCacheClearPrompt from "./ShelfChapterCacheClearPrompt.vue";
import SettingsDetailHeader from "./SettingsDetailHeader.vue";

const route = useRoute();
const reader = getReaderSession();
const bootstrapped = ref(false);
const chapterCacheUsage = ref<import("../../api/types").ChapterCacheUsage | null>(null);
const chapterCacheUsageBusy = ref(false);
const chapterCacheUsageError = ref("");
const pendingShelfChapterCacheClear = ref(false);
const shelfChapterCacheClearBusy = ref(false);
const shelfChapterCacheClearResult = ref<{
  clearedBooks: number;
  failedBooks: number;
  notStartedBooks: number;
  clearedChapters: number;
  errors: Array<{ bookId: string; title: string; message: string }>;
} | null>(null);
const pendingBookChapterCacheClear = ref<{ id: string; title: string } | null>(null);
const bookChapterCacheClearBusy = ref(false);
const bookChapterCacheClearError = ref("");
const screen = computed(() => String(route.meta.screen ?? "home"));
const cacheActions = useChapterCacheActions({
  readingBook: reader.readingBook,
  readerVisible: reader.readerVisible,
  readerProgressDirty: reader.readerProgressDirty,
  chapterCacheUsage,
  chapterCacheUsageBusy,
  chapterCacheUsageError,
  pendingShelfClear: pendingShelfChapterCacheClear,
  shelfClearBusy: shelfChapterCacheClearBusy,
  shelfClearResult: shelfChapterCacheClearResult,
  pendingBookClear: pendingBookChapterCacheClear,
  bookClearBusy: bookChapterCacheClearBusy,
  bookClearError: bookChapterCacheClearError,
  screen,
  bootstrapped,
  hasConflictingOperation: () => backupBusy.value !== null,
  getRestoreRevision: getFrontendRestoreRevision,
  saveCurrentProgress: reader.saveCurrentProgress,
  finishReadingSession: reader.finishReadingSession,
  closeReader: reader.closeReader,
  errorText,
  notify,
});
const cacheRefreshBlocked = computed(() => chapterCacheUsageBusy.value || shelfChapterCacheClearBusy.value
  || bookChapterCacheClearBusy.value || backupBusy.value !== null || shelfBatchRecoveryRequired.value);
const cacheClearBlocked = computed(() => cacheRefreshBlocked.value);
const formatChapterCacheBytes = cacheActions.formatChapterCacheBytes;
onMounted(async () => {
  try {
    bootstrapped.value = true;
    void cacheActions.refreshChapterCacheUsage();
  } catch (error) {
    notify(`读取正文缓存页面失败：${errorText(error)}`, "error");
  }
});

</script>

<template>
  <section class="settings-detail-page">
    <SettingsDetailHeader title="正文缓存" />
    <div class="settings-detail-content">
      <div class="cache-summary">
        <div class="cache-summary-copy">
          <span class="cache-caption">总占用</span>
          <template v-if="chapterCacheUsage">
            <strong>{{ formatChapterCacheBytes(chapterCacheUsage.chapterResourceBytes) }}</strong>
            <small class="cache-caption">{{ chapterCacheUsage.chapterResourceCount }} 章</small>
          </template>
          <p v-else class="cache-caption">{{ chapterCacheUsageBusy ? '正在读取…' : '暂无统计' }}</p>
        </div>
        <button type="button" class="cache-refresh" :disabled="cacheRefreshBlocked" @click="cacheActions.refreshChapterCacheUsage">
          <PrototypeIcon name="refresh" />{{ chapterCacheUsageBusy ? '读取中…' : '刷新' }}
        </button>
      </div>
      <dl v-if="chapterCacheUsage" class="cache-breakdown">
        <div>
          <dt>在线章节 <small>{{ chapterCacheUsage.onlineChapterResourceCount }} 章</small></dt>
          <dd>{{ formatChapterCacheBytes(chapterCacheUsage.onlineChapterResourceBytes) }}</dd>
        </div>
        <div>
          <dt>本地章节 <small>{{ chapterCacheUsage.localChapterResourceCount }} 章</small></dt>
          <dd>{{ formatChapterCacheBytes(chapterCacheUsage.localChapterResourceBytes) }}</dd>
        </div>
      </dl>
      <p v-if="chapterCacheUsageError" class="cache-error" role="alert">{{ chapterCacheUsageError }}</p>
      <div class="cache-actions">
        <button type="button" class="cache-clear" :disabled="cacheClearBlocked" @click="pendingShelfChapterCacheClear = true">
          <PrototypeIcon name="trash" />{{ shelfChapterCacheClearBusy ? '清理中…' : '清理在线缓存' }}
        </button>
      </div>
      <div v-if="shelfChapterCacheClearResult" class="cache-result" role="status">
        <p>
          已清理 {{ shelfChapterCacheClearResult.clearedBooks }} 本书的 {{ shelfChapterCacheClearResult.clearedChapters }} 章缓存
          <template v-if="shelfChapterCacheClearResult.failedBooks">，{{ shelfChapterCacheClearResult.failedBooks }} 本失败</template>
          <template v-if="shelfChapterCacheClearResult.notStartedBooks">，{{ shelfChapterCacheClearResult.notStartedBooks }} 本未开始</template>
        </p>
        <details v-if="shelfChapterCacheClearResult.errors.length">
          <summary>查看详情</summary>
          <ul><li v-for="error in shelfChapterCacheClearResult.errors" :key="error.bookId">{{ error.title }}：{{ error.message }}</li></ul>
        </details>
      </div>
    </div>
  </section>
  <ShelfChapterCacheClearPrompt
    :open="pendingShelfChapterCacheClear"
    :busy="shelfChapterCacheClearBusy"
    :locked="shelfBatchRecoveryRequired || backupBusy !== null"
    :clear="cacheActions.clearShelfOnlineChapterCaches"
    :cancel="() => { pendingShelfChapterCacheClear = false }"
  />
</template>
<style scoped>
.settings-detail-page {
  width: 100%;
  min-width: 0;
  min-height: 100dvh;
  color: var(--app-text);
}
.settings-detail-content {
  width: min(640px, 100%);
  margin: 0 auto;
  padding: var(--ui-space-6) var(--ui-page-gutter) var(--ui-space-8);
}

.cache-summary {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ui-space-4);
  padding-bottom: var(--ui-space-5);
}

.cache-summary-copy {
  display: grid;
  gap: var(--ui-space-1);
  min-width: 0;
}

.cache-summary-copy strong {
  font-size: var(--ui-font-title);
  font-weight: 650;
  line-height: 1.3;
  font-variant-numeric: tabular-nums;
}

.cache-caption {
  margin: 0;
  color: var(--app-muted);
  font-size: var(--ui-font-caption);
}

.cache-refresh,
.cache-clear {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex: none;
  gap: var(--ui-space-2);
  min-height: var(--ui-control-height);
  padding: 0 var(--ui-space-3);
  border: 1px solid transparent;
  border-radius: var(--ui-radius);
  background: transparent;
  font-size: var(--ui-font-body);
  white-space: nowrap;
  cursor: pointer;
}

.cache-refresh {
  color: var(--app-muted);
}

.cache-refresh :deep(svg),
.cache-clear :deep(svg) {
  width: 18px;
  height: 18px;
}

.cache-refresh:focus-visible,
.cache-clear:focus-visible,
.cache-result summary:focus-visible {
  outline: 2px solid var(--app-accent);
  outline-offset: 2px;
}

.cache-breakdown {
  margin: 0;
  border-top: 1px solid var(--app-line);
}

.cache-breakdown > div {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ui-space-3);
  min-height: 52px;
  padding-block: var(--ui-space-3);
  border-bottom: 1px solid var(--app-line);
  font-size: var(--ui-font-body);
}

.cache-breakdown dt {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: var(--ui-space-2);
  min-width: 0;
}

.cache-breakdown small {
  color: var(--app-muted);
  font-size: var(--ui-font-caption);
}

.cache-breakdown dd {
  flex: none;
  margin: 0;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}

.cache-actions {
  padding-top: var(--ui-space-5);
}

.cache-clear {
  border-color: var(--app-danger-line);
  color: var(--app-danger);
}

.cache-error,
.cache-result {
  margin: var(--ui-space-4) 0 0;
  font-size: var(--ui-font-caption);
  overflow-wrap: anywhere;
}

.cache-error {
  color: var(--app-danger);
}

.cache-result {
  color: var(--app-muted);
}

.cache-result p {
  margin: 0;
}

.cache-result details {
  margin-top: var(--ui-space-2);
}

.cache-result summary {
  cursor: pointer;
}

.cache-result ul {
  margin: var(--ui-space-2) 0 0;
  padding-left: var(--ui-space-5);
}
</style>

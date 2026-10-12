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
        <section class="settings-detail-panel">
          <header class="settings-detail-section-header"><strong>本地正文资源</strong></header>
          <div v-if="chapterCacheUsage" class="settings-cache-hero">
            <span class="settings-cache-hero-icon"><PrototypeIcon name="download"/></span>
            <div><strong>{{ formatChapterCacheBytes(chapterCacheUsage.chapterResourceBytes) }}</strong><small>{{ chapterCacheUsage.chapterResourceCount }} 个缓存章节</small></div>
            <button type="button" class="settings-detail-reload" :disabled="cacheRefreshBlocked" @click="cacheActions.refreshChapterCacheUsage"><PrototypeIcon name="refresh"/> 刷新</button>
          </div>
          <template v-if="chapterCacheUsage">
            <div class="setting-control"><div><label>全部章节</label><small>{{ chapterCacheUsage.chapterResourceCount }} 个缓存项</small></div><strong>{{ formatChapterCacheBytes(chapterCacheUsage.chapterResourceBytes) }}</strong></div>
            <div class="setting-control"><div><label>在线章节</label><small>{{ chapterCacheUsage.onlineChapterResourceCount }} 个</small></div><strong>{{ formatChapterCacheBytes(chapterCacheUsage.onlineChapterResourceBytes) }}</strong></div>
            <div class="setting-control"><div><label>本地章节</label><small>{{ chapterCacheUsage.localChapterResourceCount }} 个</small></div><strong>{{ formatChapterCacheBytes(chapterCacheUsage.localChapterResourceBytes) }}</strong></div>
          </template>
          <p v-else class="settings-detail-note">{{ chapterCacheUsageBusy ? '正在读取缓存占用…' : '尚未读取缓存占用。' }}</p>
          <p v-if="chapterCacheUsageError" class="settings-detail-error">{{ chapterCacheUsageError }}</p>
          <div v-if="shelfChapterCacheClearResult" class="settings-detail-note">
            <strong>成功 {{ shelfChapterCacheClearResult.clearedBooks }} 本，失败 {{ shelfChapterCacheClearResult.failedBooks }} 本，未开始 {{ shelfChapterCacheClearResult.notStartedBooks }} 本</strong>
            <ul v-if="shelfChapterCacheClearResult.errors.length"><li v-for="error in shelfChapterCacheClearResult.errors" :key="error.bookId">{{ error.title }}：{{ error.message }}</li></ul>
          </div>
          <div class="settings-detail-actions">
            <button type="button" class="button secondary" :disabled="cacheRefreshBlocked" @click="cacheActions.refreshChapterCacheUsage">{{ chapterCacheUsageBusy ? '读取中…' : '重新统计' }}</button>
            <button type="button" class="button danger" :disabled="cacheClearBlocked" @click="pendingShelfChapterCacheClear = true">{{ shelfChapterCacheClearBusy ? '清理中…' : '清理在线章节缓存' }}</button>
          </div>
        </section>
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
  width: min(820px, 100%);
  min-height: 100dvh;
  margin: 0 auto;
  padding: 10px 0 50px;
  color: var(--app-text);
}
.settings-detail-content {
  display: grid;
  gap: 18px;
}

.settings-detail-panel {
  padding: 0;
  border: 0;
  border-radius: 0;
  background: transparent;
}

.settings-detail-section-header {
  display: grid;
  gap: 4px;
  margin: 0 0 9px;
  padding: 0 2px 12px;
  border-bottom: 1px solid var(--app-line);
}

.settings-detail-section-header strong {
  color: var(--app-text);
  font-size: var(--ui-font-caption);
}

.settings-detail-panel .setting-control {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  min-height: 68px;
  padding: 8px 4px;
  border-bottom: 1px solid var(--app-line);
}

.settings-detail-panel .setting-control > div {
  display: grid;
  gap: 4px;
  min-width: 0;
}

.settings-detail-panel .setting-control label {
  color: var(--app-text);
  font-size: var(--ui-font-body);
  font-weight: 650;
}

.settings-detail-panel .setting-control small,
.settings-detail-note {
  color: var(--app-subtle);
  font-size: var(--ui-font-caption);
  line-height: 1.5;
}

.settings-detail-panel .setting-control select {
  min-width: 105px;
  min-height: 36px;
  padding: 5px;
  border: 1px solid var(--app-line);
  border-radius: 8px;
  background: var(--app-surface);
}
 
@media (max-width: 640px) {
  .settings-detail-content { gap: 22px; }
  .settings-detail-panel .setting-control {
    min-height: 65px;
    align-items: center;
  }
  .settings-detail-panel .setting-control > div {
    flex: 1;
    max-width: 60%;
  }
}


.settings-detail-actions {
  display: flex;
  flex-wrap: wrap;
  justify-content: flex-end;
  gap: 8px;
  padding-top: 15px;
}

.settings-detail-actions .button {
  min-height: 37px;
  border-radius: 9px;
  font-size: var(--ui-font-body);
}

.settings-detail-error {
  padding: 9px 11px;
  border-radius: 9px;
  color: var(--app-danger);
  font-size: var(--ui-font-caption);
  background: var(--app-danger-soft);
}

.settings-cache-hero {
  display: grid;
  grid-template-columns: 44px minmax(0, 1fr) auto;
  align-items: center;
  gap: 12px;
  margin: 15px 0 7px;
  padding: 17px;
  border: 1px solid var(--app-line);
  border-radius: 14px;
  background: var(--app-surface);
}

.settings-cache-hero-icon {
  display: grid;
  place-items: center;
  width: 43px;
  height: 43px;
  border-radius: 12px;
  color: var(--app-accent);
  background: var(--app-accent-soft);
}

.settings-cache-hero-icon .prototype-icon {
  width: 23px;
  height: 23px;
}

.settings-cache-hero > div {
  display: grid;
  gap: 3px;
  min-width: 0;
}

.settings-cache-hero strong {
  color: var(--app-text);
  font-size: var(--ui-font-title);
  line-height: 1.25;
}

.settings-cache-hero small {
  color: var(--app-subtle);
  font-size: var(--ui-font-caption);
}

.settings-detail-reload {
  display: flex;
  align-items: center;
  gap: 5px;
  min-height: 35px;
  padding: 0 9px;
  border: 0;
  border-radius: 9px;
  color: var(--app-muted);
  font-size: var(--ui-font-caption);
  background: var(--app-panel-soft);
  cursor: pointer;
}

.settings-detail-reload:disabled {
  opacity: .4;
  cursor: default;
}

.settings-detail-reload .prototype-icon {
  width: 15px;
  height: 15px;
}
@media (max-width: 640px) {
  .settings-cache-hero {
    grid-template-columns: 37px minmax(0, 1fr) auto;
    gap: 8px;
    padding: 12px;
  }
  .settings-cache-hero-icon { width: 37px; height: 37px; }
  .settings-cache-hero strong { font-size: var(--ui-font-heading); }
  .settings-detail-reload { padding: 0 7px; font-size: var(--ui-font-caption); }
}

</style>

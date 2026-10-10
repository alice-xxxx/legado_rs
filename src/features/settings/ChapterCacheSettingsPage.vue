<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, shallowRef } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useRoute } from "vue-router";
import { appBootstrap } from "../../api/core";
import { readResource } from "../../api/resources";
import type { AppBootstrap, ResourceDescriptor, ShelfResource } from "../../api/types";
import { errorText, notify } from "../../app/notifications";
import {
  getFrontendRestoreRevision,
  shelfBatchRecoveryMessage,
  shelfBatchRecoveryRequired,
  shelfBatchRecoveryTitle,
} from "../../app/recoveryState";
import { openedBook } from "../books/bookDetailState";
import { getReaderSession } from "../reader/readerSession";
import { backupBusy } from "./backupState";
import { useChapterCacheActions } from "./useChapterCacheActions";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import ShelfChapterCacheClearPrompt from "./ShelfChapterCacheClearPrompt.vue";
import SettingsDetailHeader from "./SettingsDetailHeader.vue";

const route = useRoute();
const reader = getReaderSession();
const shelf = shallowRef<ShelfResource>({ schemaVersion: 1, books: [] });
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
  shelf,
  openedBook,
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
  recoveryRequired: shelfBatchRecoveryRequired,
  recoveryTitle: shelfBatchRecoveryTitle,
  recoveryMessage: shelfBatchRecoveryMessage,
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
const unlisteners: UnlistenFn[] = [];
let disposed = false;
let shelfReadRevision = 0;

async function refreshShelf(descriptor?: ResourceDescriptor): Promise<void> {
  const restoreRevision = getFrontendRestoreRevision();
  const requestRevision = ++shelfReadRevision;
  const source = descriptor ?? (await appBootstrap()).shelf;
  const next = await readResource<ShelfResource>(source);
  if (disposed || restoreRevision !== getFrontendRestoreRevision() || requestRevision !== shelfReadRevision) return;
  shelf.value = { ...next, groups: next.groups ?? [], books: next.books ?? [] };
}

function retain(unlisten: UnlistenFn): void {
  if (disposed) unlisten();
  else unlisteners.push(unlisten);
}

async function setupEvents(): Promise<void> {
  try {
    retain(await listen<ResourceDescriptor>("shelf-updated", (event) => refreshShelf(event.payload).catch((error) => {
      notify(`书架更新失败：${errorText(error)}`, "error");
    })));
    retain(await listen<AppBootstrap>("app-state-updated", async (event) => {
      const revision = getFrontendRestoreRevision();
      cacheActions.resetForRestore();
      bootstrapped.value = false;
      try {
        await refreshShelf(event.payload.shelf);
        if (revision === getFrontendRestoreRevision()) bootstrapped.value = true;
      } catch (error) {
        if (revision === getFrontendRestoreRevision()) notify(`恢复后的书架读取失败：${errorText(error)}`, "error");
      }
    }));
  } catch (error) {
    if (!disposed) console.debug("Chapter cache event subscription unavailable:", errorText(error));
  }
}

onMounted(async () => {
  void setupEvents();
  try {
    await refreshShelf();
    bootstrapped.value = true;
    void cacheActions.refreshChapterCacheUsage();
  } catch (error) {
    notify(`读取正文缓存页面失败：${errorText(error)}`, "error");
  }
});

onBeforeUnmount(() => {
  disposed = true;
  shelfReadRevision += 1;
  for (const unlisten of unlisteners.splice(0)) unlisten();
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
  color: #373a45;
  font-size: 13px;
}

.settings-detail-panel .setting-control {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  min-height: 68px;
  padding: 8px 4px;
  border-bottom: 1px solid #e8eaf0;
}

.settings-detail-panel .setting-control > div {
  display: grid;
  gap: 4px;
  min-width: 0;
}

.settings-detail-panel .setting-control label {
  color: #383a45;
  font-size: 13px;
  font-weight: 650;
}

.settings-detail-panel .setting-control small,
.settings-detail-note {
  color: #9196a1;
  font-size: 11px;
  line-height: 1.5;
}

.settings-detail-panel .setting-control select {
  min-width: 105px;
  min-height: 36px;
  padding: 5px;
  border: 1px solid #e4e5ec;
  border-radius: 8px;
  background: #fff;
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
  font-size: 12px;
}

.settings-detail-error {
  padding: 9px 11px;
  border-radius: 9px;
  color: #aa4b56;
  font-size: 12px;
  background: #fff1f2;
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
  background: #fff;
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
  color: #343641;
  font-size: 21px;
  line-height: 1.25;
}

.settings-cache-hero small {
  color: #989da6;
  font-size: 11px;
}

.settings-detail-reload {
  display: flex;
  align-items: center;
  gap: 5px;
  min-height: 35px;
  padding: 0 9px;
  border: 0;
  border-radius: 9px;
  color: #60646d;
  font-size: 12px;
  background: #eceef2;
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
  .settings-cache-hero strong { font-size: 17px; }
  .settings-detail-reload { padding: 0 7px; font-size: 11px; }
}

</style>

<script setup lang="ts">
import { Searchbar as KSearchbar, Dialog as KDialog, DialogButton as KDialogButton, Checkbox as KCheckbox, Button as KButton } from "konsta/vue";
import SubpageHeader from "../../ui/SubpageHeader.vue";
// 完整书签直接保存在全局 history 中。
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { bookPreviews, loadBookPreviews } from "../books/bookDetailState";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { notify } from "../../app/notifications";
import type { AppBootstrap, Bookmark, HistoryResource } from "../../api/types";
import { routeNames } from "../../router";
import { returnToRoute } from "../../app/routeReturn";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import { backupBusy } from "./backupState";
import { getReaderSession } from "../reader/readerSession";
import { history as appHistory, removeHistoryBookmark } from "./historyStore";

const mutationBlocked = computed(() => backupBusy.value !== null || shelfBatchRecoveryRequired.value);

async function openBookmark(bookmark: Bookmark): Promise<void> {
  if (mutationBlocked.value) return;
  try {
    await getReaderSession().openBookmark(bookmark);
  } catch (error) {
    notify(error instanceof Error ? error.message : String(error), "error");
  }
}


const bookmarks = ref<Bookmark[]>([]);
const loading = ref(true);
const loadError = ref("");
const shelfError = ref("");
const bookmarkResourceError = ref("");
const bookmarkDeleteBusy = ref(false);
const bookmarkDeleteError = ref("");
const bookmarkVisibleLimit = ref(120);
const bookmarkSearchOpen = ref(false);
const bookmarkManagementOpen = ref(false);
const bookmarkQuery = ref("");
const selectedBookmarkIds = ref<string[]>([]);
const pendingBookmarkDeleteIds = ref<string[] | null>(null);
const bookmarkDeleteFeedback = ref("");

let disposed = false;
let restoreRevision = 0;
let loadRevision = 0;
let bookmarkReadRevision = 0;
const unlisteners: UnlistenFn[] = [];

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function applyBookmarkRecords(value: Bookmark[]): void {
  bookmarks.value = value;
  const currentIds = new Set(bookmarks.value.map((bookmark) => bookmark.id));
  selectedBookmarkIds.value = selectedBookmarkIds.value.filter((id) => currentIds.has(id));
}

async function refreshBookmarkRecords(): Promise<void> {
  const requestRevision = ++bookmarkReadRevision;
  const currentRestoreRevision = restoreRevision;
  try {
    const records = loadBookmarkRecords(appHistory.value);
    if (disposed || currentRestoreRevision !== restoreRevision || requestRevision !== bookmarkReadRevision) return;
    applyBookmarkRecords(records);
    bookmarkResourceError.value = "";
  } catch (error) {
    if (!disposed && currentRestoreRevision === restoreRevision && requestRevision === bookmarkReadRevision) {
      bookmarkResourceError.value = errorText(error);
    }
  }
}

function loadBookmarkRecords(history: HistoryResource): Bookmark[] {
  return Object.entries(history.bookmarks)
    .flatMap(([bookId, records]) => records.map((bookmark) => ({ ...bookmark, bookId })));
}

async function loadPageData(_snapshot?: AppBootstrap): Promise<void> {
  const requestRevision = ++loadRevision;
  const currentRestoreRevision = restoreRevision;
  const bookmarkReadAtStart = bookmarkReadRevision;
  loading.value = true;
  loadError.value = "";
  try {
    const nextBookmarks = loadBookmarkRecords(appHistory.value);
    if (disposed || currentRestoreRevision !== restoreRevision || requestRevision !== loadRevision) return;
    if (bookmarkReadAtStart === bookmarkReadRevision) {
      bookmarkReadRevision += 1;
      applyBookmarkRecords(nextBookmarks);
      bookmarkResourceError.value = "";
    }
  } catch (error) {
    if (!disposed && currentRestoreRevision === restoreRevision && requestRevision === loadRevision) {
      loadError.value = errorText(error);
    }
  } finally {
    if (!disposed && currentRestoreRevision === restoreRevision && requestRevision === loadRevision) loading.value = false;
  }
}

function handleRestore(snapshot: AppBootstrap): void {
  restoreRevision += 1;
  loadRevision += 1;
  bookmarkReadRevision += 1;
  selectedBookmarkIds.value = [];
  pendingBookmarkDeleteIds.value = null;
  bookmarkDeleteFeedback.value = "";
  bookmarkDeleteError.value = "";
  void loadPageData(snapshot);
}

async function deleteSelectedBookmarks(): Promise<void> {
  const requestedIds = pendingBookmarkDeleteIds.value;
  if (mutationBlocked.value || loading.value || bookmarkDeleteBusy.value || !requestedIds?.length) return;
  const ids = [...new Set(requestedIds)].filter((id) => bookmarks.value.some((bookmark) => bookmark.id === id));
  if (!ids.length) {
    pendingBookmarkDeleteIds.value = null;
    return;
  }

  const currentRestoreRevision = restoreRevision;
  bookmarkDeleteBusy.value = true;
  bookmarkDeleteError.value = "";
  bookmarkDeleteFeedback.value = "";
  const deletedIds: string[] = [];
  const failedIds: string[] = [];
  let interruptedByBlock = false;
  try {
    for (const id of ids) {
      if (disposed || currentRestoreRevision !== restoreRevision) return;
      if (mutationBlocked.value) {
        interruptedByBlock = true;
        break;
      }
      const bookmark = bookmarks.value.find((entry) => entry.id === id);
      if (!bookmark) continue;
      try {
        await removeHistoryBookmark(bookmark.bookId, id);
      } catch (error) {
        failedIds.push(id);
        bookmarkDeleteError.value = errorText(error);
        continue;
      }
      deletedIds.push(id);
    }
    if (disposed || currentRestoreRevision !== restoreRevision) return;
    pendingBookmarkDeleteIds.value = null;
    const deletedSet = new Set(deletedIds);
    selectedBookmarkIds.value = ids.filter((id) => !deletedSet.has(id));
    bookmarkDeleteFeedback.value = interruptedByBlock
      ? `操作已暂停，已删除 ${deletedIds.length} 个书签；其余项目仍保留选择。`
      : failedIds.length
        ? `已删除 ${deletedIds.length} 个书签，${failedIds.length} 个失败；失败项仍保留选择，可重试。`
        : `已删除 ${deletedIds.length} 个书签。`;
  } finally {
    bookmarkDeleteBusy.value = false;
  }
}

watch(() => Object.keys(appHistory.value.bookmarks), (ids) => { void loadBookPreviews(ids); }, { immediate: true });
function coverForBook(bookId: string): string | undefined { return bookPreviews.value[bookId]?.coverSrc; }
function title(bookId: string): string { return bookPreviews.value[bookId]?.title ?? "缓存书信息暂不可用"; }

const filteredBookmarks = computed(() => {
  const query = bookmarkQuery.value.trim().toLocaleLowerCase();
  return [...bookmarks.value]
    .sort((a, b) => b.updatedAtMs - a.updatedAtMs)
    .filter((bookmark) => !query || [
      title(bookmark.bookId),
      bookmark.chapterTitle ?? "",
      `第 ${bookmark.chapterIndex + 1} 章`,
      bookmark.note,
    ].some((value) => value.toLocaleLowerCase().includes(query)));
});
const visibleBookmarks = computed(() => filteredBookmarks.value.slice(0, bookmarkVisibleLimit.value));
const matchingBookmarksPerBook = computed(() => {
  const counts = new Map<string, number>();
  for (const bookmark of filteredBookmarks.value) counts.set(bookmark.bookId, (counts.get(bookmark.bookId) ?? 0) + 1);
  return counts;
});
watch(bookmarkQuery, () => { bookmarkVisibleLimit.value = 120; });
const groupedBookmarks = computed(() => {
  const groups: Array<{ bookId: string; items: Bookmark[] }> = [];
  const byBook = new Map<string, { bookId: string; items: Bookmark[] }>();
  for (const bookmark of visibleBookmarks.value) {
    let group = byBook.get(bookmark.bookId);
    if (!group) {
      group = { bookId: bookmark.bookId, items: [] };
      byBook.set(bookmark.bookId, group);
      groups.push(group);
    }
    group.items.push(bookmark);
  }
  return groups;
});
const selectedBookmarks = computed(() => bookmarks.value.filter((bookmark) => selectedBookmarkIds.value.includes(bookmark.id)));
const allVisibleBookmarksSelected = computed(() => visibleBookmarks.value.length > 0
  && visibleBookmarks.value.every((bookmark) => selectedBookmarkIds.value.includes(bookmark.id)));

function toggleBookmarkSelection(bookmarkId: string): void {
  bookmarkDeleteFeedback.value = "";
  selectedBookmarkIds.value = selectedBookmarkIds.value.includes(bookmarkId)
    ? selectedBookmarkIds.value.filter((id) => id !== bookmarkId)
    : [...selectedBookmarkIds.value, bookmarkId];
}

function toggleVisibleBookmarkSelection(): void {
  bookmarkDeleteFeedback.value = "";
  const visibleIds = visibleBookmarks.value.map((bookmark) => bookmark.id);
  if (allVisibleBookmarksSelected.value) {
    selectedBookmarkIds.value = selectedBookmarkIds.value.filter((id) => !visibleIds.includes(id));
  } else {
    selectedBookmarkIds.value = [...new Set([...selectedBookmarkIds.value, ...visibleIds])];
  }
}

function bookmarkUpdatedLabel(timestamp: number): string {
  const date = new Date(timestamp);
  if (!Number.isFinite(date.getTime())) return "日期未知";
  const today = new Date();
  const eventDay = new Date(date.getFullYear(), date.getMonth(), date.getDate());
  const nowDay = new Date(today.getFullYear(), today.getMonth(), today.getDate());
  const dayGap = Math.round((Date.UTC(nowDay.getFullYear(), nowDay.getMonth(), nowDay.getDate())
    - Date.UTC(eventDay.getFullYear(), eventDay.getMonth(), eventDay.getDate())) / 86_400_000);
  const time = date.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit", hour12: false });
  if (dayGap === 0) return `今天 ${time}`;
  if (dayGap === 1) return `昨天 ${time}`;
  return date.toLocaleDateString("zh-CN", { month: "numeric", day: "numeric" });
}

function closeBookmarkSearch(): void {
  bookmarkSearchOpen.value = !bookmarkSearchOpen.value;
  if (!bookmarkSearchOpen.value) bookmarkQuery.value = "";
}

function returnToSettings(): void {
  returnToRoute(routeNames.settings);
}

async function setupEvents(): Promise<void> {
  try {
    const unlistenRestore = await listen<AppBootstrap>("app-state-updated", (event) => handleRestore(event.payload));
    if (disposed) { unlistenRestore(); return; }
    unlisteners.push(unlistenRestore);
  } catch (error) {
    console.debug("Native bookmark listeners unavailable:", errorText(error));
  }
}

onMounted(async () => {
  await setupEvents();
  await loadPageData();
});

watch(() => appHistory.value.bookmarks, () => {
  void refreshBookmarkRecords().catch((error: unknown) => {
    if (!disposed) bookmarkResourceError.value = errorText(error);
  });
}, { deep: true });

onBeforeUnmount(() => {
  disposed = true;
  restoreRevision += 1;
  loadRevision += 1;
  bookmarkReadRevision += 1;
  for (const unlisten of unlisteners.splice(0)) unlisten();
});
</script>

<template>
  <SubpageHeader class="history-topbar col-span-full row-start-1" title="书签" back-label="返回我的" @back="returnToSettings" >
<template #actions><KButton rounded inline clear v-if="bookmarks.length" type="button" class="record-icon-button gap-2 size-11 p-0"
      aria-label="搜索书签"
      :disabled="mutationBlocked || loading || bookmarkDeleteBusy" @click="closeBookmarkSearch"><PrototypeIcon name="search"/></KButton></template>
</SubpageHeader>
  <section class="my-records my-records--bookmarks">
    <p v-if="loading" class="record-state" role="status">正在加载书签…</p>
    <div v-else-if="loadError" class="record-state record-error" role="alert">
      书签载入失败：{{ loadError }} <KButton rounded inline clear class="gap-2" type="button" @click="loadPageData()">重试</KButton>
    </div>
    <template v-else>
      <p v-if="shelfError" class="record-state record-error" role="alert">书架更新失败：{{ shelfError }} <KButton rounded inline clear class="gap-2" type="button" @click="loadPageData()">重试</KButton></p>
      <p v-if="bookmarkResourceError" class="record-state record-error" role="alert">书签更新失败：{{ bookmarkResourceError }} <KButton rounded inline clear class="gap-2" type="button" @click="loadPageData()">重试</KButton></p>
      <p v-if="bookmarkDeleteError" class="record-state record-error" role="alert">删除失败：{{ bookmarkDeleteError }}</p>
      <div v-if="bookmarks.length" class="bookmarks-toolbar">
        <span class="bookmark-total">{{ bookmarks.length }} 个书签</span>
        <KButton rounded inline clear type="button" class="record-action gap-2" :disabled="mutationBlocked || bookmarkDeleteBusy"
          @click="bookmarkManagementOpen = !bookmarkManagementOpen; selectedBookmarkIds = []; pendingBookmarkDeleteIds = null">
          {{ bookmarkManagementOpen ? '完成' : '管理' }}
        </KButton>
      </div>
      <KSearchbar v-if="bookmarkSearchOpen" class="record-search p-0!" aria-label="搜索书签" placeholder="搜索书名、章节或备注"
        :inert="mutationBlocked || bookmarkDeleteBusy" :disable-button="false" :value="bookmarkQuery"
        @input="bookmarkQuery = ($event.target as HTMLInputElement).value" @clear="bookmarkQuery = ''" />
      <div v-if="bookmarks.length && bookmarkManagementOpen" class="bookmark-operations">
        <label class="bookmark-select-all"><KCheckbox component="span" :checked="allVisibleBookmarksSelected"
          :disabled="mutationBlocked || bookmarkDeleteBusy || !visibleBookmarks.length" @change="toggleVisibleBookmarkSelection" />选择当前结果</label>
        <span v-if="selectedBookmarks.length">已选 {{ selectedBookmarks.length }} 项</span>
        <KButton rounded inline clear v-if="selectedBookmarks.length" class="record-action gap-2" type="button" :disabled="mutationBlocked || bookmarkDeleteBusy"
          @click="selectedBookmarkIds = []; bookmarkDeleteFeedback = ''; pendingBookmarkDeleteIds = null">取消选择</KButton>
        <KButton rounded inline clear v-if="selectedBookmarks.length" type="button" class="record-action record-delete k-color-brand-danger gap-2 " :disabled="mutationBlocked || bookmarkDeleteBusy"
          @click="pendingBookmarkDeleteIds = selectedBookmarks.map(bookmark => bookmark.id); bookmarkDeleteFeedback = ''">删除所选</KButton>
      </div>
      <p v-if="bookmarkDeleteFeedback" class="record-feedback">{{ bookmarkDeleteFeedback }}</p>
      <div v-if="visibleBookmarks.length" class="bookmark-groups">
        <section v-for="group in groupedBookmarks" :key="group.bookId" class="bookmark-group">
          <header>
            <span class="record-book-icon bookmark-group-cover">
              <img v-if="coverForBook(group.bookId)" :src="coverForBook(group.bookId)" loading="lazy" decoding="async"/>
              <span v-else class="record-cover-placeholder"></span>
            </span>
            <div><strong>{{ title(group.bookId) }}</strong><small>{{ matchingBookmarksPerBook.get(group.bookId) ?? group.items.length }} 个书签</small></div>
          </header>
          <div v-for="bookmark in group.items" :key="bookmark.id" class="bookmark-entry">
            <KCheckbox component="span" v-if="bookmarkManagementOpen" class="bookmark-select" :checked="selectedBookmarkIds.includes(bookmark.id)"
              :disabled="mutationBlocked || bookmarkDeleteBusy"
              @change="toggleBookmarkSelection(bookmark.id)" />
            <button type="button" class="bookmark-target"
              :disabled="bookmark.orphaned || mutationBlocked || bookmarkDeleteBusy" @click="openBookmark(bookmark)">
              <span class="bookmark-anchor">{{ bookmark.chapterTitle || '第 ' + (bookmark.chapterIndex + 1) + ' 章' }}</span>
              <span><strong>{{ bookmark.note || (bookmark.orphaned ? '旧章节书签，无法定位' : '返回保存的阅读位置') }}</strong>
                <small>{{ bookmark.orphaned ? '原章节不可用' : bookmarkUpdatedLabel(bookmark.updatedAtMs) }}</small></span>
              <PrototypeIcon name="right"/>
            </button>
            <KButton rounded inline clear v-if="bookmarkManagementOpen" type="button" class="record-action record-delete k-color-brand-danger gap-2 " :disabled="mutationBlocked || bookmarkDeleteBusy"
              @click="pendingBookmarkDeleteIds = [bookmark.id]; bookmarkDeleteFeedback = ''">删除</KButton>
          </div>
        </section>
      </div>
      <div v-else class="record-state">{{ bookmarks.length ? '没有匹配的书签。' : '阅读时添加书签，就能从这里回到保存的位置。' }}</div>
      <KButton rounded inline clear v-if="filteredBookmarks.length > visibleBookmarks.length" type="button" class="record-load-more gap-2"
        @click="bookmarkVisibleLimit += 120">显示更多书签 · {{ visibleBookmarks.length }} / {{ filteredBookmarks.length }}</KButton>
      <div v-if="pendingBookmarkDeleteIds?.length" class="bookmark-confirm-backdrop" data-back-layer>
        <KDialog opened :backdrop="false" class="bookmark-confirm w-[min(420px,calc(100vw-32px))] max-h-[88dvh] overflow-auto">
  <template #title><h2>{{ pendingBookmarkDeleteIds.length === 1 ? '删除这个书签？' : '删除所选书签？' }}</h2></template>
<p>将删除 {{ pendingBookmarkDeleteIds.length }} 个书签，删除后无法撤销。</p><template #buttons><KDialogButton type="button" data-back-action :disabled="bookmarkDeleteBusy" @click="pendingBookmarkDeleteIds = null">取消</KDialogButton>
<KDialogButton type="button" :disabled="mutationBlocked || bookmarkDeleteBusy" @click="deleteSelectedBookmarks" class="k-color-brand-danger" strong>{{ bookmarkDeleteBusy ? '删除中…' : '确认删除' }}</KDialogButton></template>
</KDialog>
      </div>
    </template>
  </section>
</template>
<style scoped>
@layer components {
.my-records--bookmarks{box-sizing:border-box;width:min(var(--app-content-width),100%);margin:0 auto;padding:24px var(--ui-page-gutter) 32px;color:var(--app-text)}
.bookmarks-toolbar{display:flex;align-items:center;justify-content:space-between;min-height:30px;margin:0 0 14px}
.bookmark-total{margin-right:auto;font-size:var(--ui-font-caption);color:var(--app-subtle)}
.record-action{padding:7px 9px}
.bookmarks-toolbar .record-action{min-height:30px}
.record-search{width:min(420px,100%);margin:0 0 13px}
.bookmark-operations{display:flex;align-items:center;justify-content:flex-start;flex-wrap:wrap;gap:10px;margin-bottom:14px;padding-bottom:11px;border-bottom:1px solid var(--app-line);font-size:var(--ui-font-caption);color:var(--app-subtle)}
.bookmark-groups{display:grid;gap:14px}
.bookmark-group{overflow:hidden;border:1px solid var(--app-line);border-radius:14px;background:var(--app-surface)}
.bookmark-group>header{box-sizing:border-box;display:flex;align-items:center;gap:10px;min-height:66px;padding:10px 12px;border-bottom:1px solid var(--app-line)}
.bookmark-group>header>div{display:flex;flex-direction:column;gap:2px;min-width:0}
.bookmark-group>header strong{font-size:var(--ui-font-body);font-weight:750;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.bookmark-group>header small{font-size:var(--ui-font-caption);color:var(--app-subtle)}
.record-book-icon{display:grid;place-items:center;flex:none;width:45px;height:54px;border-radius:7px;background:#546c72;color:#fff;overflow:hidden}
.record-book-icon img{display:block;width:100%;height:100%;border-radius:inherit;object-fit:cover}
.record-cover-placeholder{position:relative;display:block;width:100%;height:100%;background:linear-gradient(140deg,#506f73,#253c42)}
.record-cover-placeholder::after{content:"";position:absolute;top:15%;right:-25%;height:70%;width:75%;border-radius:50%;background:#ffffff25}
.bookmark-group-cover{width:34px;height:44px;border-radius:6px}
.bookmark-entry{box-sizing:border-box;display:flex;align-items:center;gap:8px;min-height:50px;padding:0 12px;border-bottom:1px solid var(--app-line)}
.bookmark-entry:last-child{border-bottom:0}
.bookmark-target{box-sizing:border-box;min-height:50px;width:100%;display:grid;grid-template-columns:76px minmax(0,1fr) 16px;align-items:center;gap:8px;padding:0;border:0;background:transparent;text-align:left;cursor:pointer}
.bookmark-target:disabled{opacity:.55;cursor:default}
.bookmark-anchor{display:block;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:var(--app-accent-ink);font-size:var(--ui-font-caption)}
.bookmark-target>span:nth-child(2){display:flex;flex-direction:column;gap:3px;min-width:0}
.bookmark-target strong{font-size:var(--ui-font-body);color:var(--app-text);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.bookmark-target small{font-size:var(--ui-font-caption);color:var(--app-subtle)}
.bookmark-target>.prototype-icon{grid-column:3;grid-row:1;width:16px;height:16px;color:var(--app-subtle)}
.bookmark-entry>.record-action{flex:none}
.record-feedback{font-size:var(--ui-font-caption);color:var(--app-accent)}
.record-state{display:flex;align-items:center;justify-content:center;min-height:105px;padding:20px 0;color:var(--app-subtle);font-size:var(--ui-font-caption);text-align:center}
.record-load-more{display:block;margin:18px auto 9px;padding:10px 16px}
.bookmark-confirm-backdrop{position:fixed;inset:0;z-index:80;display:grid;place-items:center;padding:14px;background:var(--app-scrim)}
.record-icon-button{display:grid;place-items:center;width:35px;height:35px}
.record-icon-button svg{width:19px;height:19px}
@media(max-width:660px){
  .my-records--bookmarks{padding:24px var(--ui-page-gutter) 32px}
  .bookmark-target{grid-template-columns:70px minmax(0,1fr) 16px;gap:7px}
}
@media(max-width:400px){
  .bookmark-target{grid-template-columns:58px minmax(0,1fr) 16px;gap:6px}
}
}
</style>

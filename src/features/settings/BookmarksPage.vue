<script setup lang="ts">
// 页面自行读取书签和书架资源；书签删除由 Rust 命令持久化。
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { appBootstrap } from "../../api/core";
import { readResource } from "../../api/resources";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { notify } from "../../app/notifications";
import { deleteBookmark, listBookmarks } from "../../api/reading";
import type { AppBootstrap, Bookmark, BookmarksResource, ResourceDescriptor, ShelfResource } from "../../api/types";
import router, { routeNames } from "../../router";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import BackButton from "../../ui/BackButton.vue";
import { backupBusy } from "./backupState";
import { getReaderSession } from "../reader/readerSession";

const mutationBlocked = computed(() => backupBusy.value !== null || shelfBatchRecoveryRequired.value);

async function openBookmark(bookmark: Bookmark): Promise<void> {
  if (mutationBlocked.value) return;
  try {
    await getReaderSession().openBookmark(bookmark);
  } catch (error) {
    notify(error instanceof Error ? error.message : String(error), "error");
  }
}

const shelf = ref<ShelfResource>({ schemaVersion: 1, books: [], groups: [] });
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
let shelfReadRevision = 0;
let bookmarkReadRevision = 0;
let shelfEventRevision = 0;
let bookmarkEventRevision = 0;
const unlisteners: UnlistenFn[] = [];

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function normalizeShelf(value: ShelfResource): ShelfResource {
  return { ...value, groups: value.groups ?? [], books: value.books ?? [] };
}

function applyBookmarkDocument(value: BookmarksResource): void {
  bookmarks.value = Array.isArray(value.bookmarks) ? value.bookmarks : [];
  const currentIds = new Set(bookmarks.value.map((bookmark) => bookmark.id));
  selectedBookmarkIds.value = selectedBookmarkIds.value.filter((id) => currentIds.has(id));
}

async function applyShelfDescriptor(descriptor: ResourceDescriptor): Promise<void> {
  const requestRevision = ++shelfReadRevision;
  const currentRestoreRevision = restoreRevision;
  const next = normalizeShelf(await readResource<ShelfResource>(descriptor));
  if (disposed || currentRestoreRevision !== restoreRevision || requestRevision !== shelfReadRevision) return;
  shelf.value = next;
  shelfError.value = "";
}

async function applyBookmarkDescriptor(descriptor: ResourceDescriptor): Promise<void> {
  const requestRevision = ++bookmarkReadRevision;
  const currentRestoreRevision = restoreRevision;
  try {
    const next = await readResource<BookmarksResource>(descriptor);
    if (disposed || currentRestoreRevision !== restoreRevision || requestRevision !== bookmarkReadRevision) return;
    applyBookmarkDocument(next);
    bookmarkResourceError.value = "";
  } catch (error) {
    if (!disposed && currentRestoreRevision === restoreRevision && requestRevision === bookmarkReadRevision) {
      bookmarkResourceError.value = errorText(error);
    }
  }
}

async function loadPageData(snapshot?: AppBootstrap): Promise<void> {
  const requestRevision = ++loadRevision;
  const currentRestoreRevision = restoreRevision;
  const shelfEventAtStart = shelfEventRevision;
  const bookmarkEventAtStart = bookmarkEventRevision;
  loading.value = true;
  loadError.value = "";
  try {
    const bootstrap = snapshot ?? await appBootstrap();
    const bookmarksDescriptor = await listBookmarks();
    const [nextShelf, nextBookmarks] = await Promise.all([
      readResource<ShelfResource>(bootstrap.shelf),
      readResource<BookmarksResource>(bookmarksDescriptor),
    ]);
    if (disposed || currentRestoreRevision !== restoreRevision || requestRevision !== loadRevision) return;
    if (shelfEventAtStart === shelfEventRevision) {
      shelfReadRevision += 1;
      shelf.value = normalizeShelf(nextShelf);
      shelfError.value = "";
    }
    if (bookmarkEventAtStart === bookmarkEventRevision) {
      bookmarkReadRevision += 1;
      applyBookmarkDocument(nextBookmarks);
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
  shelfReadRevision += 1;
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
      const eventAtStart = bookmarkEventRevision;
      let descriptor: ResourceDescriptor;
      try {
        descriptor = await deleteBookmark(id);
      } catch (error) {
        failedIds.push(id);
        bookmarkDeleteError.value = errorText(error);
        continue;
      }
      deletedIds.push(id);
      try {
        const next = await readResource<BookmarksResource>(descriptor);
        if (!disposed && currentRestoreRevision === restoreRevision && eventAtStart === bookmarkEventRevision) {
          bookmarkReadRevision += 1;
          applyBookmarkDocument(next);
          bookmarkResourceError.value = "";
        }
      } catch (error) {
        if (!disposed && currentRestoreRevision === restoreRevision) bookmarkResourceError.value = errorText(error);
      }
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

const shelfBookMap = computed(() => new Map(shelf.value.books.map((book) => [book.id, book])));
const bookCoverMap = computed(() => new Map(shelf.value.books.map((book) => [book.id, book.coverSrc])));
function coverForBook(bookId: string): string | undefined { return bookCoverMap.value.get(bookId); }
function title(bookId: string): string { return shelfBookMap.value.get(bookId)?.title ?? "书架中已移除的书"; }

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
  void router.push({ name: routeNames.settings });
}

async function setupEvents(): Promise<void> {
  try {
    const unlistenShelf = await listen<ResourceDescriptor>("shelf-updated", (event) => {
      shelfEventRevision += 1;
      void applyShelfDescriptor(event.payload).catch((error: unknown) => {
        if (!disposed) shelfError.value = errorText(error);
      });
    });
    if (disposed) { unlistenShelf(); return; }
    unlisteners.push(unlistenShelf);
    const unlistenResources = await listen<{ kind: string; resource: ResourceDescriptor }>("resource-updated", (event) => {
      if (event.payload.kind !== "bookmarks") return;
      bookmarkEventRevision += 1;
      void applyBookmarkDescriptor(event.payload.resource);
    });
    if (disposed) { unlistenResources(); return; }
    unlisteners.push(unlistenResources);
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

onBeforeUnmount(() => {
  disposed = true;
  restoreRevision += 1;
  loadRevision += 1;
  shelfReadRevision += 1;
  bookmarkReadRevision += 1;
  for (const unlisten of unlisteners.splice(0)) unlisten();
});
</script>

<template>
  <header class="history-topbar bookmarks-topbar">
    <BackButton label="返回我的" @click="returnToSettings" />
    <div class="history-heading"><strong>书签</strong></div>
    <button v-if="bookmarks.length" type="button" class="record-icon-button"
      :disabled="mutationBlocked || loading || bookmarkDeleteBusy" @click="closeBookmarkSearch"><PrototypeIcon name="search"/></button>
    <span v-else></span>
  </header>
  <section class="my-records my-records--bookmarks">
    <p v-if="loading" class="record-state" role="status">正在加载书签…</p>
    <div v-else-if="loadError" class="record-state record-error" role="alert">
      书签载入失败：{{ loadError }} <button type="button" @click="loadPageData()">重试</button>
    </div>
    <template v-else>
      <p v-if="shelfError" class="record-state record-error" role="alert">书架更新失败：{{ shelfError }} <button type="button" @click="loadPageData()">重试</button></p>
      <p v-if="bookmarkResourceError" class="record-state record-error" role="alert">书签更新失败：{{ bookmarkResourceError }} <button type="button" @click="loadPageData()">重试</button></p>
      <p v-if="bookmarkDeleteError" class="record-state record-error" role="alert">删除失败：{{ bookmarkDeleteError }}</p>
      <div v-if="bookmarks.length" class="bookmarks-toolbar">
        <span class="bookmark-total">{{ bookmarks.length }} 个书签</span>
        <button type="button" class="record-action" :disabled="mutationBlocked || bookmarkDeleteBusy"
          @click="bookmarkManagementOpen = !bookmarkManagementOpen; selectedBookmarkIds = []; pendingBookmarkDeleteIds = null">
          {{ bookmarkManagementOpen ? '完成' : '管理' }}
        </button>
      </div>
      <label v-if="bookmarkSearchOpen" class="record-search"><PrototypeIcon name="search"/>
        <input v-model="bookmarkQuery" type="search" placeholder="搜索书名、章节或备注" :disabled="mutationBlocked || bookmarkDeleteBusy"/>
      </label>
      <div v-if="bookmarks.length && bookmarkManagementOpen" class="bookmark-operations">
        <label class="bookmark-select-all"><input type="checkbox" :checked="allVisibleBookmarksSelected"
          :disabled="mutationBlocked || bookmarkDeleteBusy || !visibleBookmarks.length" @change="toggleVisibleBookmarkSelection"/>选择当前结果</label>
        <span v-if="selectedBookmarks.length">已选 {{ selectedBookmarks.length }} 项</span>
        <button v-if="selectedBookmarks.length" class="record-action" type="button" :disabled="mutationBlocked || bookmarkDeleteBusy"
          @click="selectedBookmarkIds = []; bookmarkDeleteFeedback = ''; pendingBookmarkDeleteIds = null">取消选择</button>
        <button v-if="selectedBookmarks.length" type="button" class="record-action record-delete" :disabled="mutationBlocked || bookmarkDeleteBusy"
          @click="pendingBookmarkDeleteIds = selectedBookmarks.map(bookmark => bookmark.id); bookmarkDeleteFeedback = ''">删除所选</button>
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
            <input v-if="bookmarkManagementOpen" type="checkbox" class="bookmark-select" :checked="selectedBookmarkIds.includes(bookmark.id)"
              :disabled="mutationBlocked || bookmarkDeleteBusy"
              @change="toggleBookmarkSelection(bookmark.id)"/>
            <button type="button" class="bookmark-target"
              :disabled="bookmark.orphaned || mutationBlocked || bookmarkDeleteBusy" @click="openBookmark(bookmark)">
              <span class="bookmark-anchor">{{ bookmark.chapterTitle || '第 ' + (bookmark.chapterIndex + 1) + ' 章' }}</span>
              <span><strong>{{ bookmark.note || (bookmark.orphaned ? '旧章节书签，无法定位' : '返回保存的阅读位置') }}</strong>
                <small>{{ bookmark.orphaned ? '原章节不可用' : bookmarkUpdatedLabel(bookmark.updatedAtMs) }}</small></span>
              <PrototypeIcon name="right"/>
            </button>
            <button v-if="bookmarkManagementOpen" type="button" class="record-action record-delete" :disabled="mutationBlocked || bookmarkDeleteBusy"
              @click="pendingBookmarkDeleteIds = [bookmark.id]; bookmarkDeleteFeedback = ''">删除</button>
          </div>
        </section>
      </div>
      <div v-else class="record-state">{{ bookmarks.length ? '没有匹配的书签。' : '阅读时添加书签，就能从这里回到保存的位置。' }}</div>
      <button v-if="filteredBookmarks.length > visibleBookmarks.length" type="button" class="record-load-more"
        @click="bookmarkVisibleLimit += 120">显示更多书签 · {{ visibleBookmarks.length }} / {{ filteredBookmarks.length }}</button>
      <div v-if="pendingBookmarkDeleteIds?.length" class="bookmark-confirm-backdrop">
        <div class="bookmark-confirm">
          <strong>{{ pendingBookmarkDeleteIds.length === 1 ? '删除这个书签？' : '删除所选书签？' }}</strong>
          <p>将删除 {{ pendingBookmarkDeleteIds.length }} 个书签，删除后无法撤销。</p>
          <footer><button type="button" class="record-action" :disabled="bookmarkDeleteBusy" @click="pendingBookmarkDeleteIds = null">取消</button>
            <button type="button" class="record-action record-delete" :disabled="mutationBlocked || bookmarkDeleteBusy"
              @click="deleteSelectedBookmarks">{{ bookmarkDeleteBusy ? '删除中…' : '确认删除' }}</button></footer>
        </div>
      </div>
    </template>
  </section>
</template>
<style scoped>
.my-records--bookmarks{box-sizing:border-box;width:min(820px,calc(100% - 36px));margin:0 auto;padding:24px 0 54px;color:#343641}
.history-topbar{position:sticky;top:0;z-index:35;height:64px;box-sizing:border-box;display:grid;grid-template-columns:44px minmax(0,1fr) 42px;align-items:center;gap:10px;padding:0 max(18px,calc((100% - 1180px) / 2));border-bottom:1px solid var(--app-line);background:rgb(246 247 249 / 92%);backdrop-filter:blur(18px)}
.history-heading{display:flex;flex-direction:column;gap:2px;min-width:0}
.history-heading strong{font-size:14px;font-weight:750;color:#18191d;line-height:1.25}
.bookmarks-topbar{grid-template-columns:44px minmax(0,1fr) 42px}
.bookmarks-topbar .record-icon-button{margin:0;width:36px;height:36px}
.bookmarks-toolbar{display:flex;align-items:center;justify-content:space-between;min-height:30px;margin:0 0 14px}
.bookmark-total{margin-right:auto;font-size:9px;color:#999da6}
.record-action{padding:7px 9px;border:0;border-radius:8px;background:transparent;color:var(--app-accent);font-size:12px;cursor:pointer}
.record-action:hover:not(:disabled){background:#f0edff}
.record-action:disabled{opacity:.5;cursor:default}
.bookmarks-toolbar .record-action{min-height:30px;font-size:10px}
.record-delete{color:#ae565e}
.record-search{display:flex;align-items:center;gap:9px;box-sizing:border-box;width:min(420px,100%);height:40px;margin:0 0 13px;padding:0 11px;border:1px solid #e3e5ed;border-radius:10px;background:#fff}
.record-search>.prototype-icon{width:18px;height:18px;color:#9497a2}
.record-search input{flex:1;min-width:0;border:0;outline:0;color:#323641;background:transparent;font-size:13px}
.bookmark-operations{display:flex;align-items:center;justify-content:flex-start;flex-wrap:wrap;gap:10px;margin-bottom:14px;padding-bottom:11px;border-bottom:1px solid #e8e9ee;font-size:12px;color:#969ba6}
.bookmark-select-all{display:flex;align-items:center;gap:8px}
.bookmark-operations input,.bookmark-select{width:17px;height:17px;accent-color:var(--app-accent)}
.bookmark-groups{display:grid;gap:14px}
.bookmark-group{overflow:hidden;border:1px solid var(--app-line);border-radius:14px;background:#fff}
.bookmark-group>header{box-sizing:border-box;display:flex;align-items:center;gap:10px;min-height:66px;padding:10px 12px;border-bottom:1px solid var(--app-line)}
.bookmark-group>header>div{display:flex;flex-direction:column;gap:2px;min-width:0}
.bookmark-group>header strong{font-size:10px;font-weight:750;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.bookmark-group>header small{font-size:8px;color:#9a9ea6}
.record-book-icon{display:grid;place-items:center;flex:none;width:45px;height:54px;border-radius:7px;background:#546c72;color:#fff;overflow:hidden}
.record-book-icon img{display:block;width:100%;height:100%;border-radius:inherit;object-fit:cover}
.record-cover-placeholder{position:relative;display:block;width:100%;height:100%;background:linear-gradient(140deg,#506f73,#253c42)}
.record-cover-placeholder::after{content:"";position:absolute;top:15%;right:-25%;height:70%;width:75%;border-radius:50%;background:#ffffff25}
.bookmark-group-cover{width:34px;height:44px;border-radius:6px}
.bookmark-entry{box-sizing:border-box;display:flex;align-items:center;gap:8px;min-height:50px;padding:0 12px;border-bottom:1px solid #eff0f3}
.bookmark-entry:last-child{border-bottom:0}
.bookmark-target{box-sizing:border-box;min-height:50px;width:100%;display:grid;grid-template-columns:76px minmax(0,1fr) 16px;align-items:center;gap:8px;padding:0;border:0;background:transparent;text-align:left;cursor:pointer}
.bookmark-target:disabled{opacity:.55;cursor:default}
.bookmark-anchor{display:block;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:#655a89;font-size:8px}
.bookmark-target>span:nth-child(2){display:flex;flex-direction:column;gap:3px;min-width:0}
.bookmark-target strong{font-size:9px;color:#343640;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.bookmark-target small{font-size:7px;color:#9a9ea6}
.bookmark-target>.prototype-icon{grid-column:3;grid-row:1;width:16px;height:16px;color:#afb4bc}
.bookmark-select{flex:none;width:15px;height:15px}
.bookmark-entry>.record-action{flex:none;font-size:10px}
.record-feedback{font-size:12px;color:var(--app-accent)}
.record-state{display:flex;align-items:center;justify-content:center;min-height:105px;padding:20px 0;color:#9296a2;font-size:13px;text-align:center}
.record-load-more{display:block;margin:18px auto 9px;padding:10px 16px;border:1px solid #e3def7;border-radius:10px;background:#f7f5ff;color:var(--app-accent);font-size:12px;cursor:pointer}
.record-load-more:hover{background:#eeeafb}
.bookmark-confirm-backdrop{position:fixed;inset:0;z-index:80;display:grid;place-items:center;padding:14px;background:#1d1d2b66}
.bookmark-confirm{box-sizing:border-box;width:min(420px,100%);padding:22px;border-radius:15px;background:#fff;box-shadow:0 15px 45px #11122224}
.bookmark-confirm strong{font-size:16px}
.bookmark-confirm p{font-size:13px;color:#767c89}
.bookmark-confirm footer{display:flex;justify-content:flex-end;gap:12px;margin-top:17px}
.record-icon-button{display:grid;place-items:center;width:35px;height:35px;border:0;border-radius:9px;background:transparent;color:#7c818e;cursor:pointer}
.record-icon-button svg{width:19px;height:19px}
.record-icon-button:hover{background:#eeecf8;color:var(--app-accent)}
@media(max-width:660px){
  .history-topbar{height:64px;padding:0 18px}
  .my-records--bookmarks{width:calc(100% - 36px);padding:24px 0 54px}
  .bookmark-target{grid-template-columns:70px minmax(0,1fr) 16px;gap:7px}
}
@media(max-width:400px){
  .bookmark-target{grid-template-columns:58px minmax(0,1fr) 16px;gap:6px}
}
</style>

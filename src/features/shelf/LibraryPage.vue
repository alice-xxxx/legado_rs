<script setup lang="ts">
import { ListInput as KListInput, Checkbox as KCheckbox, Button as KButton, Glass as KGlass } from "konsta/vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { computed, onBeforeUnmount, onMounted, reactive, ref } from "vue";
import { onBeforeRouteLeave, useRoute } from "vue-router";
import type { ShelfSortKey, ShelfSortOrder } from "../../api/types";
import { bookContentType } from "../../api/bookContentType";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { notify } from "../../app/notifications";
import { contentLibrary, librarySort, librarySortOrder } from "../books/contentLibraryState";
import { shelfBooks as shelf } from "./shelfBooks";
import { openedBookFromShelf, switchOpenedBook } from "../books/bookDetailState";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import router, { routeNames } from "../../router";
import PrimaryNavigation from "../../ui/PrimaryNavigation.vue";
import SubpageHeader from "../../ui/SubpageHeader.vue";
import SelectMenu from "../../ui/SelectMenu.vue";
import LibraryBookInfo from "./LibraryBookInfo.vue";
import { getReaderSession } from "../reader/readerSession";
import { discardBookDetailOpenRequest, requestBookDetailOpen } from "../books/bookDetailOpenRequest";
import ShelfOrganizer from "./ShelfOrganizer.vue";
import { useShelfBatchActions } from "./useShelfBatchActions";
import { useShelfGroupActions } from "./useShelfGroupActions";

const route = useRoute();
const readerSession = getReaderSession();
const activeGroup = ref("");
const mutationLocked = shelfBatchRecoveryRequired;
let restoreRevision = 0;
const unlisteners: UnlistenFn[] = [];

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

async function continueReading(bookId: string): Promise<void> {
  const exists = shelf.value.books.some(entry => entry.id === bookId);
  if (!exists) {
    notify("书籍文件暂时不可用。", "error");
    return;
  }
  try {
    openedBookFromShelf.value = true;
    const book = await switchOpenedBook(bookId);
    if (book) await readerSession.startReading(book);
  } catch (error) {
    notify(`打开阅读失败：${errorText(error)}`, "error");
  }
}

async function openBookInfo(bookId: string): Promise<void> {
  if (mutationLocked.value) return;
  const requestId = requestBookDetailOpen({ kind: "shelf-book", bookId });
  try {
    const failure = await router.push({ name: routeNames.bookDetail });
    if (failure) discardBookDetailOpenRequest(requestId);
  } catch (error) {
    discardBookDetailOpenRequest(requestId);
    notify(errorText(error), "error");
  }
}

async function setupEvents(): Promise<void> {
  try {
    unlisteners.push(await listen("app-state-updated", () => {
      restoreRevision += 1;
    }));
  } catch (error) {
    // Browser preview does not provide native events; command results remain authoritative.
    console.debug("Native shelf event subscription unavailable:", errorText(error));
  }
}

const visibleBooks = computed(() => {
  if (!activeGroup.value) return shelf.value.books;
  const group = contentLibrary.value.groups?.find(group => group.name === activeGroup.value);
  if (!group) return [];
  if (librarySort.value !== "custom") return shelf.value.books.filter(book => group.id.includes(book.id));
  const books = new Map(shelf.value.books.map(book => [book.id, book]));
  return group.id.flatMap(id => books.has(id) ? [books.get(id)!] : []);
});
const groups = computed(() => shelf.value.groups ?? []);
const shelfSortKey = computed<ShelfSortKey>(() => shelf.value.sort ?? "updatedAt");
const shelfSortOrder = computed<ShelfSortOrder>(() => shelf.value.sortOrder ?? "descending");

function changeShelfSort(key: ShelfSortKey, order = shelfSortOrder.value): void {
  if (mutationLocked.value) return;
  librarySort.value = key;
  librarySortOrder.value = order;
}

const shelfBatch = reactive(useShelfBatchActions({
  visibleBooks,
  groups,
  recoveryRequired: mutationLocked,
  isShelfScreen: () => route.name === routeNames.shelf,
  getRestoreRevision: () => restoreRevision,
  notify,
  errorText,
}));

const shelfGroup = reactive({
  ...useShelfGroupActions({
    groups,
    activeGroup,
    selectionMode: computed(() => shelfBatch.selectionMode),
    batchBusy: computed(() => shelfBatch.batchBusy),
    toggleBookSelection: shelfBatch.toggleBookSelection,
    openBookInfo,
    continueReading,
    notify,
  }),
  shelfGroups: groups,
  shelfActiveGroup: activeGroup,
});

const shelfView = reactive({
  shelf,
  visibleBooks,
  shelfSortKey,
  shelfSortOrder,
  changeShelfSort,
});

onBeforeRouteLeave(() => !shelfBatch.batchBusy);

onMounted(setupEvents);

onBeforeUnmount(() => {
  restoreRevision += 1;
  for (const unlisten of unlisteners.splice(0)) unlisten();
});

type LibraryType = "all" | "text" | "html" | "image" | "video" | "audio" | "pdf";
const typeFilters: { key: LibraryType; title: string }[] = [
  { key: "all", title: "全部" },
  { key: "text", title: "文本" },
  { key: "audio", title: "音频" },
  { key: "video", title: "视频" },
  { key: "image", title: "图片" },
  { key: "html", title: "HTML" },
  { key: "pdf", title: "PDF" },
];
const activeType = ref<LibraryType>("all");
const manageGroupsOpen = ref(false);

const displayedBooks = computed(() => shelfBatch.selectionMode || activeType.value === "all"
  ? shelfView.visibleBooks
  : shelfView.visibleBooks.filter(book => (bookContentType(book) ?? "text") === activeType.value));

function enterManageMode(): void {
  // Start management with the complete shelf.
  shelfGroup.shelfActiveGroup = "";
  manageGroupsOpen.value = false;
  shelfBatch.enterSelectionMode();
}

function exitManageMode(): void {
  if (shelfBatch.batchBusy) return;
  manageGroupsOpen.value = false;
  shelfGroup.shelfActiveGroup = "";
  shelfBatch.exitSelectionMode();
}

function openImport(): void {
  void router.push({ name: routeNames.shelfImport });
}

function groupBookCount(group: string): number {
  return group ? shelfView.shelf.books.filter((book) => book.groups?.includes(group)).length
    : shelfView.shelf.books.length;
}
</script>

<template>
  <div class="shelf-page-layout" :class="{ 'library-manage-page': shelfBatch.selectionMode }" data-back-page>
    <PrimaryNavigation v-if="!shelfBatch.selectionMode" />
    <main class="shelf-page-main">
      <SubpageHeader v-if="!shelfBatch.selectionMode" title="内容库" :back="false" large>
        <template #actions>
          <KButton rounded inline clear class="size-11 p-0" type="button" aria-label="导入内容" title="导入 TXT / EPUB / CBZ / PDF" :disabled="mutationLocked" @click="openImport"><PrototypeIcon name="import" /></KButton>
          <KButton rounded inline clear class="size-11 p-0" type="button" aria-label="整理内容库" title="整理内容库" :disabled="shelfBatch.batchBusy || mutationLocked" @click="enterManageMode"><PrototypeIcon name="tune" /></KButton>
        </template>
      </SubpageHeader>
      <section class="shelf-view ui-page-content" :class="{ 'library-manage-mode': shelfBatch.selectionMode, 'shelf-view-empty': !shelfBatch.selectionMode && !displayedBooks.length }">
        <SubpageHeader v-if="shelfBatch.selectionMode" title="整理内容库" subtitle="分组与批量管理" back-label="退出管理" local-back :back-disabled="shelfBatch.batchBusy" @back="exitManageMode">
          <template #actions>
          <KButton rounded inline clear type="button" class="library-done-button gap-2" :disabled="shelfBatch.batchBusy"
            @click.stop="exitManageMode()">完成</KButton>
          </template>
        </SubpageHeader>
        <div v-else class="library-topbar">
          <nav class="library-type-filter" aria-label="内容类型">
            <KButton rounded :tonal="activeType === filter.key" :clear="!(activeType === filter.key)" inline class="gap-2" v-for="filter in typeFilters" :key="filter.key" type="button"
              :aria-pressed="activeType === filter.key" @click.stop="activeType = filter.key">{{ filter.title
              }}</KButton>
          </nav>
        </div>
        <div v-if="shelfBatch.selectionMode" class="library-manage-workspace">
          <aside class="library-manage-rail">
            <nav class="library-manage-group-list">
              <KButton rounded :tonal="!shelfGroup.shelfActiveGroup" :clear="!(!shelfGroup.shelfActiveGroup)" inline class="gap-2" type="button" :class="{ active: !shelfGroup.shelfActiveGroup }" :disabled="shelfBatch.batchBusy"
                @click.stop="shelfGroup.shelfActiveGroup = ''">
                <span>全部内容</span><small>{{ groupBookCount('') }}</small>
              </KButton>
              <KButton rounded :tonal="shelfGroup.shelfActiveGroup === group" :clear="!(shelfGroup.shelfActiveGroup === group)" inline class="gap-2" v-for="group in shelfGroup.shelfGroups" :key="group" type="button"
                :class="{ active: shelfGroup.shelfActiveGroup === group }" :disabled="shelfBatch.batchBusy"
                @click.stop="shelfGroup.shelfActiveGroup = group">
                <span>{{ group }}</span><small>{{ groupBookCount(group) }}</small>
              </KButton>
            </nav>
            <KButton rounded inline clear type="button" class="library-manage-groups-button gap-2"
              :disabled="shelfBatch.batchBusy || mutationLocked" @click.stop="manageGroupsOpen = !manageGroupsOpen">
              {{ manageGroupsOpen ? '收起分组编辑' : '编辑分组' }}
            </KButton>
            <ShelfOrganizer v-if="manageGroupsOpen" :groups="shelfGroup.shelfGroups"
              :active-group="shelfGroup.shelfActiveGroup" :manage-groups-open="true"
              @filter-group="shelfGroup.shelfActiveGroup = $event" @create-group="shelfGroup.createGroup"
              @rename-group="shelfGroup.renameGroup" @delete-group="shelfGroup.removeShelfGroup" />
          </aside>
          <main class="library-manage-content">
            <div class="shelf-batch-toolbar library-manage-selectbar">
              <label class="library-select-all">
                <KCheckbox component="span" :checked="shelfBatch.allVisibleBooksSelected"
                  :disabled="shelfBatch.batchBusy || !shelfView.visibleBooks.length"
                  @change="shelfBatch.allVisibleBooksSelected ? shelfBatch.clearVisibleBookSelection() : shelfBatch.selectVisibleBooks()"  />
                <span>全选</span>
              </label>
              <strong class="library-selection-count">已选 {{ shelfBatch.selectedVisibleBookIds.length }} 项</strong>
              <div class="library-manage-primary-actions">
                <KButton rounded inline tonal type="button" class="library-move-button gap-2"
                  :disabled="shelfBatch.batchBusy || mutationLocked || !shelfBatch.selectedVisibleBookIds.length || shelfBatch.selectedVisibleBookIds.length > 256"
                  @click="shelfBatch.openGroups">移动到</KButton>
                <KButton rounded inline tonal type="button" class="library-remove-button k-color-brand-danger gap-2 "
                  :disabled="shelfBatch.batchBusy || mutationLocked || !shelfBatch.selectedVisibleBookIds.length || shelfBatch.selectedVisibleBookIds.length > 256"
                  @click="shelfBatch.openRemovalConfirm">移除</KButton>
              </div>
            </div>
            <div v-if="shelfBatch.selectedVisibleBookIds.length > 256" class="shelf-batch-error">
              每次最多处理 256 本书，请减少选择。
            </div>
            <p v-if="shelfBatch.batchError && !shelfBatch.groupsOpen" class="shelf-batch-error">{{
              shelfBatch.batchError }}</p>
            <details class="library-manage-more">
              <summary>更多批量操作</summary>
              <div class="library-manage-extra-actions">
                <KButton rounded inline clear class="gap-2" type="button" :disabled="shelfBatch.batchBusy || !shelfBatch.selectedVisibleBookIds.length"
                  @click="shelfBatch.clearVisibleBookSelection">取消本组选择</KButton>
                <KButton rounded inline clear class="gap-2" type="button"
                  :disabled="shelfBatch.batchBusy || mutationLocked || !shelfBatch.selectedVisibleBookIds.length || shelfBatch.selectedVisibleBookIds.length > 256"
                  @click="shelfBatch.refreshSelectedInformation()">更新目录</KButton>
                <KButton rounded inline clear class="gap-2" type="button"
                  :disabled="shelfBatch.batchBusy || mutationLocked || !shelfBatch.selectedVisibleBookIds.length || shelfBatch.selectedVisibleBookIds.length > 256"
                  @click="shelfBatch.cacheSelectedBooks()">下载章节</KButton>
                <div class="library-manage-sort-label">排序
                  <SelectMenu :model-value="shelfView.shelfSortKey" :disabled="shelfBatch.batchBusy || mutationLocked" label="内容排序"
                    :options="[{ value: 'updatedAt', label: '最近阅读' }, { value: 'title', label: '书名' }, { value: 'author', label: '作者' }, { value: 'latestChapter', label: '最新章节' }, { value: 'progress', label: '阅读进度' }, { value: 'custom', label: '自定义顺序' }]"
                    @update:model-value="shelfView.changeShelfSort" />
                </div>
                <KButton rounded inline clear class="gap-2" type="button" :disabled="shelfBatch.batchBusy || mutationLocked"
                  @click="shelfView.changeShelfSort(shelfView.shelfSortKey, shelfView.shelfSortOrder === 'ascending' ? 'descending' : 'ascending')">
                  {{ shelfView.shelfSortOrder === 'ascending' ? '升序' : '降序' }}
                </KButton>
                <span v-if="shelfBatch.selectedLocalBooks.length">所选包含 {{ shelfBatch.selectedLocalBooks.length }}
                  本本地书，不支持在线任务。</span>
              </div>
            </details>
            <section v-if="shelfBatch.cacheResult" class="shelf-batch-result-inline">
              <span class="muted">缓存完成 {{ shelfBatch.cacheResult.completed.length }} 本；失败 {{ shelfBatch.cacheResult.failed.length }} 本；本地书跳过 {{ shelfBatch.cacheResult.skipped }} 本。</span>
              <KButton rounded inline clear class="gap-2" @click="shelfBatch.cacheResult = null">关闭结果</KButton>
              <p v-for="book in shelfBatch.cacheResult.failed" :key="book.id" class="error">{{ book.title }}：{{ book.message }}</p>
            </section>
            <div v-if="shelfView.visibleBooks.length" class="library-manage-grid">
              <label v-for="book in shelfView.visibleBooks" :key="book.id" class="library-manage-book"
                :class="{ selected: shelfBatch.selectedBookIds.includes(book.id) }">
                <span class="library-manage-cover">
                  <img v-if="book.coverSrc" :src="book.coverSrc" loading="lazy" decoding="async" />
                  <span v-else class="library-abstract-cover" :class="`cover-tone-${Math.abs(book.title.length) % 5}`"></span>
                </span>
                <span class="library-manage-info">
                  <KCheckbox component="span" :checked="shelfBatch.selectedBookIds.includes(book.id)"
                    :disabled="shelfBatch.batchBusy" @change="shelfBatch.toggleBookSelection(book.id)"  />
                  <LibraryBookInfo :book="book" />
                </span>
              </label>
            </div>
            <div v-else class="page-empty-state library-manage-empty">
              <strong>{{ shelfView.shelf.books.length ? '分组暂无内容' : '暂无内容' }}</strong>
              <KButton rounded inline tonal small v-if="shelfView.shelf.books.length" type="button" class="gap-2"
                @click="shelfGroup.shelfActiveGroup = ''">查看全部</KButton>
              <KButton rounded inline tonal small v-else type="button" class="gap-2" @click="exitManageMode()">返回内容库</KButton>
            </div>
          </main>
        </div>
        <div v-if="!shelfBatch.selectionMode && displayedBooks.length" class="book-grid">
          <article v-for="book in displayedBooks" :key="book.id" class="shelf-card">
            <button class="cover-button" type="button" title="点击阅读，长按查看书籍信息"
              @pointerdown="shelfGroup.startShelfCoverLongPress(book.id, $event)"
              @pointermove="shelfGroup.trackShelfCoverLongPress" @pointerup="shelfGroup.finishShelfCoverLongPress"
              @pointercancel="shelfGroup.cancelShelfCoverLongPress"
              @pointerleave="shelfGroup.handleShelfCoverPointerLeave" @contextmenu.prevent
              @click="shelfGroup.handleShelfCoverClick(book.id)">
              <img v-if="book.coverSrc" :src="book.coverSrc" class="book-cover" loading="lazy" />
              <span v-else class="library-abstract-cover"
                :class="`cover-tone-${Math.abs(book.title.length) % 5}`"></span>
            </button>
            <button class="book-info-button" :aria-label="`查看《${book.title}》详情`" @click="openBookInfo(book.id)">
              <LibraryBookInfo :book="book" />
            </button>
          </article>
        </div>
        <div v-else-if="!shelfBatch.selectionMode && shelfView.shelf.books.length" class="page-empty-state library-filter-empty">
          <strong>暂无匹配内容</strong>
          <KButton rounded inline tonal small class="gap-2"
            @click.stop="activeType = 'all'; shelfGroup.shelfActiveGroup = ''">查看全部</KButton>
        </div>
        <div v-else-if="!shelfBatch.selectionMode" class="page-empty-state library-filter-empty">
          <span class="empty-state-glyph"><PrototypeIcon name="library" /></span>
          <strong>把喜欢的内容放在这里</strong>
          <p>导入本地文件，或从发现和搜索中添加内容。</p>
          <KButton rounded inline @click="openImport">导入内容</KButton>
        </div>
      </section>
    </main>
  </div>

  <section v-if="shelfBatch.removeConfirmOpen" class="modal-backdrop" @click.self="shelfBatch.cancelRemoval">
    <KGlass component="article" :highlight="false" class="app-modal shelf-batch-remove-modal material:bg-md-light-surface-2 material:dark:bg-md-dark-surface-2">
      <KButton rounded inline clear class="detail-close gap-2 size-11 p-0" aria-label="取消移除" data-back-action :disabled="shelfBatch.batchBusy" @click="shelfBatch.cancelRemoval"><PrototypeIcon name="close" /></KButton>
      <p class="eyebrow">批量移除</p>
      <h2 id="shelf-batch-remove-title">确认移除 {{ shelfBatch.removalTargets.length }} 本书？</h2>
      <p class="modal-description">将从书架中移除所选书籍、阅读进度、书签和本地缓存。请核对书名；取消不会修改书架。</p>
      <ul class="shelf-batch-remove-targets">
        <li v-for="book in shelfBatch.removalTargets" :key="book.id">
          <strong>{{ book.title }}</strong><span>{{ book.author || '作者未知' }}</span>
        </li>
      </ul>
      <div class="modal-actions">
        <KButton rounded inline tonal class="gap-2" :disabled="shelfBatch.batchBusy"
          @click="shelfBatch.cancelRemoval">取消</KButton>
        <KButton rounded inline class="k-color-brand-danger gap-2 " :disabled="shelfBatch.batchBusy || mutationLocked"
          @click="shelfBatch.submitRemoval">{{ shelfBatch.batchBusy ? '正在移除…' : `确认移除
          ${shelfBatch.removalTargets.length} 本书` }}</KButton>
      </div>
    </KGlass>
  </section>

  <section v-if="shelfBatch.groupsOpen" class="modal-backdrop" @click.self="shelfBatch.closeGroups">
    <KGlass component="article" :highlight="false" class="app-modal shelf-batch-group-modal material:bg-md-light-surface-2 material:dark:bg-md-dark-surface-2">
      <KButton rounded inline clear class="detail-close gap-2 size-11 p-0" aria-label="关闭分组设置" data-back-action :disabled="shelfBatch.batchBusy" @click="shelfBatch.closeGroups"><PrototypeIcon name="close" /></KButton>
      <p class="eyebrow">批量整理</p>
      <h2 id="shelf-batch-group-title">移动 {{ shelfBatch.groupBookIds.length }} 本内容至分组</h2>
      <p class="modal-description">所选分组将完整替换这些内容当前的分组，可同时指定多个分组。其他内容和空分组不受影响。</p>
      <div class="shelf-batch-group-list">
        <label v-for="group in shelfGroup.shelfGroups" :key="group" class="shelf-batch-group-option"
          :class="{ selected: shelfBatch.groupDraft.includes(group) }">
          <KCheckbox component="span" :checked="shelfBatch.groupDraft.includes(group)"
            :disabled="shelfBatch.batchBusy || mutationLocked || shelfBatch.clearGroups"
            @change="shelfBatch.toggleGroup(group)"  />
          <span>{{ group }}</span>
        </label>
        <p v-if="!shelfGroup.shelfGroups.length" class="shelf-batch-groups-empty">还没有分组。可以在下方直接添加，或选择清空分组。</p>
      </div>
      <form class="shelf-batch-new-group" @submit.prevent="shelfBatch.addGroup">
        <label for="shelf-batch-new-group-input">添加并应用一个新分组</label>
        <div>
          <KListInput class="my-0!" outline component="div" input-id="shelf-batch-new-group-input" maxlength="128" autocomplete="off" placeholder="输入分组名称" :disabled="shelfBatch.batchBusy || mutationLocked" :value="shelfBatch.newGroup" @input="shelfBatch.newGroup = ($event.target as HTMLInputElement).value" />
          <KButton rounded inline tonal small class="gap-2" type="submit"
            :disabled="shelfBatch.batchBusy || mutationLocked || !shelfBatch.newGroup.trim()">添加</KButton>
        </div>
      </form>
      <label class="shelf-batch-clear-option">
        <KCheckbox component="span" :checked="shelfBatch.clearGroups" :disabled="shelfBatch.batchBusy || mutationLocked"
          @change="shelfBatch.setClearGroups(($event.target as HTMLInputElement).checked)"  />
        <span><strong>清空所选书的全部分组</strong><small>会把所选书籍的分组完整替换为空列表。</small></span>
      </label>
      <p v-if="shelfBatch.batchError" class="shelf-batch-error shelf-batch-modal-error">{{ shelfBatch.batchError }}
      </p>
      <div class="modal-actions">
        <KButton rounded inline tonal class="gap-2" :disabled="shelfBatch.batchBusy" @click="shelfBatch.closeGroups">取消</KButton>
        <KButton rounded inline class="gap-2"
          :disabled="shelfBatch.batchBusy || mutationLocked || (!shelfBatch.groupDraft.length && !shelfBatch.clearGroups)"
          @click="shelfBatch.submitGroups">{{ shelfBatch.batchBusy ? '正在保存…' : `确认移动 ${shelfBatch.groupBookIds.length}
          项` }}</KButton>
      </div>
    </KGlass>
  </section>
</template>

<style scoped>
@layer components {
.shelf-page-layout {
  display: flex;
  width: 100%;
  height: 100dvh;
  min-height: 0;
  overflow: hidden;
  background: var(--app-background);
}

.shelf-page-main {
  flex: 1;
  min-width: 0;
  min-height: 0;
  overflow-x: hidden;
  overflow-y: auto;
  padding: 0 0 var(--safe-bottom);
}

.library-manage-page .shelf-page-main {
  padding: 0;
}

.shelf-view-empty {
  display: flex;
  flex-direction: column;
  min-height: calc(100% - 132px);
}

.shelf-view-empty .library-topbar { flex: none; }

.book-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(var(--ui-cover-min-width), 1fr));
  gap: var(--ui-grid-gap);
}

.shelf-card { min-width: 0; }

.cover-button {
  position: relative;
  display: block;
  width: 100%;
  aspect-ratio: 5 / 7;
  padding: 0;
  border: 0;
  border-radius: var(--ui-radius);
  overflow: hidden;
  cursor: pointer;
}

.book-cover, .library-manage-cover img {
  position: absolute;
  inset: 0;
  display: block;
  width: 100%;
  height: 100%;
  object-fit: contain;
  -webkit-touch-callout: none;
  -webkit-user-drag: none;
  -webkit-user-select: none;
  user-select: none;
}

.book-info-button {
  display: block;
  width: 100%;
  text-align: left;
  cursor: pointer;
}

.library-topbar { min-width: 0; margin-bottom: 24px; }

.library-type-filter {
  display: flex;
  align-items: center;
  gap: 5px;
  min-width: 0;
  overflow-x: auto;
  scrollbar-width: none;
}

.library-type-filter::-webkit-scrollbar {
  display: none;
}

.library-type-filter button { flex: none; }

.library-abstract-cover {
  position: absolute;
  inset: 0;
  display: block;
  overflow: hidden;
  border-radius: 3px;
  background: linear-gradient(135deg, #456c6a, #172e31);
}

.library-abstract-cover::before {
  content: "";
  position: absolute;
  inset: 14% -15% auto auto;
  width: 70%;
  height: 70%;
  border-radius: 50%;
  background: rgba(255, 255, 255, .13);
}

.library-abstract-cover::after {
  content: "";
  position: absolute;
  left: -20%;
  right: -20%;
  bottom: -18%;
  height: 55%;
  border-radius: 50% 50% 0 0;
  background: rgba(0, 0, 0, .14);
  transform: rotate(-8deg);
}

.library-abstract-cover.cover-tone-1 {
  background: linear-gradient(135deg, #8a6d7f, #393246);
}

.library-abstract-cover.cover-tone-2 {
  background: linear-gradient(135deg, #73939a, #2d4a45);
}

.library-abstract-cover.cover-tone-3 {
  background: linear-gradient(135deg, #384a67, #121a28);
}

.library-abstract-cover.cover-tone-4 {
  background: linear-gradient(135deg, #91677d, #362432);
}

.library-filter-empty {
  flex: 1;
  min-height: 160px;
}

@media (max-width:640px) {

  .library-topbar {
    gap: 8px;
    margin: 5px 0 18px;
  }

  .library-type-filter {
    gap: 0;
  }

  .library-type-filter button {
    padding: 0 9px;
  }

  .library-top-actions {
    gap: 4px;
  }

  .library-round-action {
    width: 36px;
    height: 36px;
  }

}

/* The selection workspace owns its own full-width layout. */
.shelf-view.library-manage-mode {
  max-width: none;
  padding: 0 0 calc(48px + var(--safe-bottom) + var(--ui-playback-inset));
  margin: 0;
  width: 100%;
}

.library-manage-workspace {
  display: grid;
  grid-template-columns: 190px minmax(0, 1fr);
  gap: 28px;
  align-items: start;
  width: min(var(--app-content-width), 100%);
  padding-inline: var(--ui-page-gutter);
  margin: 24px auto 0;
}

.library-manage-rail {
  display: flex;
  flex-direction: column;
  gap: 11px;
  min-width: 0;
}

.library-manage-group-list {
  display: grid;
  gap: 3px;
}

.library-manage-group-list button {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  min-height: 36px;
  padding: 0 9px;
  text-align: left;
}

.library-manage-group-list button.active {
  color: var(--app-text);
}

.library-manage-group-list button:disabled {
  opacity: .55;
  cursor: default;
}

.library-manage-group-list button span {
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.library-manage-group-list button small {
  color: inherit;
  font-size: var(--ui-font-caption);
  opacity: .75;
}

.library-manage-groups-button {
  align-self: flex-start;
  min-height: 28px;
  padding: 0 9px;
}

.library-manage-rail :deep(.shelf-organizer) {
  width: 100%;
  margin: 0;
  border: 0;
}

.library-manage-rail :deep(.organizer-main) {
  display: none;
}

.library-manage-rail :deep(.group-manager) {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin: 0;
  padding: 10px;
  border: 1px solid var(--app-line);
  border-radius: 10px;
}

.library-manage-rail :deep(.managed-groups) {
  display: grid;
  gap: 6px;
}

.library-manage-rail :deep(.managed-group-row) {
  display: flex;
  flex-wrap: wrap;
  min-width: 0;
}

.library-manage-rail :deep(.managed-group-row>span) {
  width: 100%;
  overflow-wrap: anywhere;
}

.library-manage-rail :deep(.new-group-form) {
  flex-wrap: wrap;
}

.library-manage-content {
  min-width: 0;
}

.library-manage-selectbar {
  display: flex;
  align-items: center;
  gap: 11px;
  min-height: 42px;
  margin: 0;
  border: 0;
  border-bottom: 1px solid var(--app-line);
  border-radius: 0;
  padding: 0 3px 8px;
  background: transparent;
  box-shadow: none;
}

.library-selection-count {
  margin-right: auto;
  color: var(--app-subtle);
  font-size: var(--ui-font-caption);
  font-weight: 500;
  white-space: nowrap;
}

.library-manage-primary-actions {
  display: flex;
  align-items: center;
  gap: 7px;
}

.library-move-button, .library-remove-button {
  min-height: 35px;
  padding: 0 12px;
}

.library-manage-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(var(--ui-cover-min-width), 1fr));
  gap: var(--ui-grid-gap);
  padding-top: 16px;
}

.library-manage-book {
  position: relative;
  display: flex;
  flex-direction: column;
  min-width: 0;
  cursor: pointer;
}

.library-manage-info {
  display: grid;
  grid-template-columns: 24px minmax(0, 1fr);
  align-items: start;
  gap: 8px;
}

.library-manage-cover {
  display: block;
  position: relative;
  aspect-ratio: 5/7;
}

.library-manage-book.selected .library-manage-cover {
  outline: 2px solid var(--app-accent);
  outline-offset: 3px;
}

.library-manage-more {
  margin-top: 9px;
  color: var(--app-muted);
  font-size: var(--ui-font-caption);
}

.library-manage-more summary {
  display: inline-block;
  padding: 6px 1px;
  cursor: pointer;
}

.library-manage-more summary::marker {
  color: var(--app-accent-ink);
}

.library-manage-extra-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  margin: 3px 0 14px;
}

.library-manage-extra-actions button {
  min-height: 30px;
  padding: 0 8px;
}

.library-manage-extra-actions button:disabled {
  opacity: .5;
  cursor: default;
}

.library-manage-extra-actions span {
  color: var(--app-danger);
  font-size: var(--ui-font-body);
}

.library-manage-sort-label {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: var(--ui-font-body);
}

.library-manage-empty {
  min-height: 220px;
}

@media (max-width:860px) {
  .shelf-page-main {
    padding: 0 0 calc(var(--ui-nav-height) + var(--safe-bottom));
  }

  .library-manage-page .shelf-page-main {
    padding: 0;
  }

  .library-manage-workspace {
    display: flex;
    flex-direction: column;
    gap: 0;
    margin: 0;
    width: 100%;
    padding-inline: 0;
  }

  .library-manage-rail {
    display: block;
    width: 100%;
    padding: 12px var(--ui-page-gutter);
    box-sizing: border-box;
  }

  .library-manage-group-list {
    display: flex;
    gap: 5px;
    overflow-x: auto;
    scrollbar-width: none;
  }

  .library-manage-group-list button {
    flex: none;
    min-height: 30px;
    padding: 0 11px;
  }

  .library-manage-groups-button {
    margin-top: 7px;
  }

  .library-manage-rail :deep(.group-manager) {
    display: grid;
    grid-template-columns: 1fr;
  }

  .library-manage-content {
    width: 100%;
    box-sizing: border-box;
    padding: 0 var(--ui-page-gutter) 35px;
  }

  .library-manage-selectbar {
    min-height: 47px;
  }

}

@media (max-width:480px) {

  .library-manage-selectbar {
    gap: 7px;
  }

  .library-manage-primary-actions {
    gap: 4px;
  }

  .library-move-button, .library-remove-button {
    min-height: 32px;
    padding: 0 9px;
  }
}
}
</style>

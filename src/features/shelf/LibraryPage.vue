<script setup lang="ts">
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { computed, onBeforeUnmount, onMounted, reactive, ref } from "vue";
import { onBeforeRouteLeave, useRoute } from "vue-router";
import { appBootstrap } from "../../api/core";
import { readResource } from "../../api/resources";
import { batchRemoveBooks, batchSetBookGroups, setShelfSort } from "../../api/shelf";
import { refreshChapters, startBookDownload, tasksResource } from "../../api/tasks";
import type { AppBootstrap, AppTask, BookResource, ResourceDescriptor, ShelfResource, ShelfSortKey, ShelfSortOrder, TasksResource } from "../../api/types";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { notify } from "../../app/notifications";
import ShelfLoadState from "./ShelfLoadState.vue";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import BackButton from "../../ui/BackButton.vue";
import router, { routeNames } from "../../router";
import PrimaryNavigation from "../../ui/PrimaryNavigation.vue";
import { readingProgressPercent } from "../books/readingProgress";
import { getReaderSession } from "../reader/readerSession";
import { discardBookDetailOpenRequest, requestBookDetailOpen } from "../books/bookDetailOpenRequest";
import ShelfOrganizer from "./ShelfOrganizer.vue";
import { useShelfBatchActions } from "./useShelfBatchActions";
import { useShelfGroupActions } from "./useShelfGroupActions";

const route = useRoute();
const readerSession = getReaderSession();
const shelf = ref<ShelfResource>({ schemaVersion: 1, books: [] });
const tasks = ref<AppTask[]>([]);
const activeGroup = ref("");
const loading = ref(true);
const loadError = ref("");
const recoveryRequired = ref(false);
const mutationLocked = computed(() => shelfBatchRecoveryRequired.value || recoveryRequired.value);
const recoveryTitle = ref("需要重启应用完成恢复");
const recoveryMessage = ref("");
let restoreRevision = 0;
let shelfReadRevision = 0;
let tasksReadRevision = 0;
const unlisteners: UnlistenFn[] = [];

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

async function continueReading(bookId: string): Promise<void> {
  const bookSrc = shelf.value.books.find((entry) => entry.id === bookId)?.bookSrc;
  if (!bookSrc) {
    notify("书籍文件暂时不可用。", "error");
    return;
  }
  try {
    await readerSession.startReading(await readResource<BookResource>(bookSrc));
  } catch (error) {
    notify(`打开阅读失败：${errorText(error)}`, "error");
  }
}

async function openBookInfo(bookId: string): Promise<void> {
  if (mutationLocked.value) return;
  const requestId = requestBookDetailOpen({ kind: "shelf-book", bookId, initialTab: "catalog" });
  try {
    const failure = await router.push({ name: routeNames.bookDetail });
    if (failure) discardBookDetailOpenRequest(requestId);
  } catch (error) {
    discardBookDetailOpenRequest(requestId);
    notify(errorText(error), "error");
  }
}

function normalizeShelf(next: ShelfResource): ShelfResource {
  return { ...next, groups: next.groups ?? [], books: next.books ?? [] };
}

async function refreshShelfFromDescriptor(descriptor: ResourceDescriptor): Promise<void> {
  const currentRestoreRevision = restoreRevision;
  const requestRevision = ++shelfReadRevision;
  const next = normalizeShelf(await readResource<ShelfResource>(descriptor));
  if (currentRestoreRevision !== restoreRevision || requestRevision !== shelfReadRevision) return;
  shelf.value = next;
}

async function refreshTasks(descriptor?: ResourceDescriptor): Promise<void> {
  const currentRestoreRevision = restoreRevision;
  const requestRevision = ++tasksReadRevision;
  const resource = descriptor ?? (await tasksResource()).resource;
  const document = await readResource<TasksResource>(resource);
  if (currentRestoreRevision !== restoreRevision || requestRevision !== tasksReadRevision) return;
  const byId = new Map((document.tasks ?? []).map((task) => [task.id, task]));
  for (const task of tasks.value) if (task.status === "recoveryRequired") byId.set(task.id, task);
  tasks.value = [...byId.values()];
  if (tasks.value.some((task) => task.status === "recoveryRequired")) {
    recoveryRequired.value = true;
    recoveryTitle.value = "目录任务需要恢复";
    recoveryMessage.value = "请重启应用完成任务恢复后，再继续修改书架。";
  }
}

async function loadShelf(): Promise<void> {
  const currentRestoreRevision = ++restoreRevision;
  const requestRevision = ++shelfReadRevision;
  tasksReadRevision += 1;
  loading.value = true;
  loadError.value = "";
  try {
    const snapshot = await appBootstrap();
    const nextShelf = normalizeShelf(await readResource<ShelfResource>(snapshot.shelf));
    if (currentRestoreRevision !== restoreRevision || requestRevision !== shelfReadRevision) return;
    shelf.value = nextShelf;
    loading.value = false;
    void refreshTasks().catch((error) => {
      if (currentRestoreRevision === restoreRevision) notify(`任务列表读取失败：${errorText(error)}`, "error");
    });
  } catch (error) {
    if (currentRestoreRevision !== restoreRevision) return;
    loadError.value = errorText(error);
  } finally {
    if (currentRestoreRevision === restoreRevision) loading.value = false;
  }
}

async function loadRestoredState(snapshot: AppBootstrap): Promise<void> {
  restoreRevision += 1;
  const currentRestoreRevision = restoreRevision;
  tasksReadRevision += 1;
  try {
    await refreshShelfFromDescriptor(snapshot.shelf);
    if (currentRestoreRevision !== restoreRevision) return;
    tasks.value = [];
    recoveryRequired.value = false;
    recoveryMessage.value = "";
    await refreshTasks();
  } catch (error) {
    if (currentRestoreRevision !== restoreRevision) return;
    recoveryRequired.value = true;
    recoveryTitle.value = "恢复后的书架载入失败";
    recoveryMessage.value = `恢复已提交，但无法完整载入书架和任务：${errorText(error)}。请重启应用。`;
    notify(recoveryMessage.value, "error");
  }
}

async function setupEvents(): Promise<void> {
  try {
    unlisteners.push(await listen<ResourceDescriptor>("shelf-updated", async (event) => {
      const currentRestoreRevision = restoreRevision;
      try {
        await refreshShelfFromDescriptor(event.payload);
      } catch (error) {
        if (currentRestoreRevision === restoreRevision) notify(`书架更新失败：${errorText(error)}`, "error");
      }
    }));
    unlisteners.push(await listen<{ kind: string; resource: ResourceDescriptor }>("resource-updated", async (event) => {
      if (event.payload.kind !== "tasks") return;
      const currentRestoreRevision = restoreRevision;
      try {
        await refreshTasks(event.payload.resource);
      } catch (error) {
        if (currentRestoreRevision === restoreRevision) notify(`任务列表更新失败：${errorText(error)}`, "error");
      }
    }));
    unlisteners.push(await listen<{ taskId: string; resource: ResourceDescriptor }>("task-updated", async (event) => {
      const currentRestoreRevision = restoreRevision;
      try {
        await refreshTasks(event.payload.resource);
      } catch (error) {
        if (currentRestoreRevision === restoreRevision) notify(`任务状态更新失败：${errorText(error)}`, "error");
      }
    }));
    unlisteners.push(await listen<Partial<AppTask>>("task-recovery-required", (event) => {
      if (event.payload.status !== "recoveryRequired") return;
      recoveryRequired.value = true;
      recoveryTitle.value = "目录任务需要恢复";
      recoveryMessage.value = event.payload.error || "请重启应用完成任务恢复后，再继续修改书架。";
    }));
    unlisteners.push(await listen<AppBootstrap>("app-state-updated", (event) => {
      void loadRestoredState(event.payload);
    }));
  } catch (error) {
    // Browser preview does not provide native events; command results remain authoritative.
    console.debug("Native shelf event subscription unavailable:", errorText(error));
  }
}

const visibleBooks = computed(() => activeGroup.value
  ? shelf.value.books.filter((book) => book.groups?.includes(activeGroup.value))
  : shelf.value.books);
const groups = computed(() => shelf.value.groups ?? []);
const shelfSortKey = computed<ShelfSortKey>(() => shelf.value.sort ?? "updatedAt");
const shelfSortOrder = computed<ShelfSortOrder>(() => shelf.value.sortOrder ?? "descending");

async function changeShelfSort(key: ShelfSortKey, order = shelfSortOrder.value): Promise<void> {
  if (mutationLocked.value) return;
  const currentRestoreRevision = restoreRevision;
  try {
    const descriptor = await setShelfSort(key, order);
    if (currentRestoreRevision !== restoreRevision) return;
    await refreshShelfFromDescriptor(descriptor);
  } catch (error) {
    if (currentRestoreRevision === restoreRevision) notify(`书架排序保存失败：${errorText(error)}`, "error");
  }
}

const shelfBatch = reactive(useShelfBatchActions({
  shelf,
  visibleBooks,
  groups,
  tasks,
  recoveryRequired,
  recoveryTitle,
  recoveryMessage,
  isShelfScreen: () => route.name === routeNames.shelf,
  getRestoreRevision: () => restoreRevision,
  refreshShelf: refreshShelfFromDescriptor,
  refreshTasks,
  refreshChapters,
  startBookDownload,
  batchRemoveBooks,
  batchSetBookGroups,
  notify,
  errorText,
}));

const shelfGroup = reactive({
  ...useShelfGroupActions({
    groups,
    activeGroup,
    selectionMode: computed(() => shelfBatch.selectionMode),
    batchBusy: computed(() => shelfBatch.batchBusy),
    refreshShelfFromDescriptor,
    getRestoreRevision: () => restoreRevision,
    toggleBookSelection: shelfBatch.toggleBookSelection,
    openBookInfo,
    continueReading,
    notify,
    errorText,
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

onMounted(() => {
  void setupEvents();
  void loadShelf();
});

onBeforeUnmount(() => {
  restoreRevision += 1;
  for (const unlisten of unlisteners.splice(0)) unlisten();
});

type LibraryType = "all" | "novel" | "comic" | "video" | "audio";
const typeFilters: { key: LibraryType; title: string }[] = [
  { key: "all", title: "全部" },
  { key: "novel", title: "小说" },
  { key: "comic", title: "漫画" },
  { key: "video", title: "视频" },
  { key: "audio", title: "音频" },
];
const activeType = ref<LibraryType>("all");
const manageGroupsOpen = ref(false);

/** 只使用 Rust 投影的书籍类型；未知类型归入小说/文本，不按书名猜测。 */
function bookType(book: ShelfResource["books"][number]): Exclude<LibraryType, "all"> {
  if (book.mediaType === "video" || book.mediaType === "audio") return book.mediaType;
  const kind = book.kind?.trim().toLowerCase() ?? "";
  if (/漫画|comic|manga|manhua|manhwa|cbz/.test(kind)) return "comic";
  if (/^(video|视频)$/.test(kind)) return "video";
  if (/^(audio|音频)$/.test(kind)) return "audio";
  return "novel";
}
const displayedBooks = computed(() => shelfBatch.selectionMode || activeType.value === "all"
  ? shelfView.visibleBooks
  : shelfView.visibleBooks.filter((book) => bookType(book) === activeType.value));

function enterManageMode(): void {
  // The prototype management workspace always starts with the complete shelf.
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

function openSearch(): void {
  void router.push({ name: routeNames.search });
}

function groupBookCount(group: string): number {
  return group ? shelfView.shelf.books.filter((book) => book.groups?.includes(group)).length
    : shelfView.shelf.books.length;
}
function chapterCountLabel(book: ShelfResource["books"][number]): string {
  return book.chapterCount ? `目录 ${book.chapterCount} 项` : "目录待更新";
}
</script>

<template>
<ShelfLoadState v-if="loading || loadError" :loading="loading" :error="loadError" :retry="loadShelf" />
<template v-else>
<div class="shelf-page-layout" :class="{ 'library-manage-page': shelfBatch.selectionMode }">
  <PrimaryNavigation v-if="!shelfBatch.selectionMode" />
  <main class="shelf-page-main">
<section class="shelf-view" :class="{ 'library-manage-mode': shelfBatch.selectionMode }">
            <header v-if="shelfBatch.selectionMode" class="library-manage-heading">
              <div class="library-manage-title">
                <BackButton label="退出管理"
                  :disabled="shelfBatch.batchBusy" @click="exitManageMode()" />
                <span><strong>整理内容库</strong><small>分组与批量管理</small></span>
              </div>
              <button type="button" class="library-done-button" :disabled="shelfBatch.batchBusy"
                @click.stop="exitManageMode()">完成</button>
            </header>
            <div v-else class="library-topbar">
              <nav class="library-type-filter">
                <button v-for="filter in typeFilters" :key="filter.key" type="button"
                  :class="{ active: activeType === filter.key }"
                  @click.stop="activeType = filter.key">{{ filter.title }}</button>
              </nav>
              <div class="library-top-actions">
                <button class="library-round-action" type="button"
title="导入 TXT / EPUB / CBZ / PDF"
                  :disabled="mutationLocked"
                  @click.stop="openImport()"><PrototypeIcon name="import"/></button>
                <button class="library-round-action" type="button"
title="整理内容库"
                  :disabled="shelfBatch.batchBusy || mutationLocked"
                  @click.stop="enterManageMode()"><PrototypeIcon name="tune"/></button>
              </div>
            </div>
            <div v-if="shelfBatch.selectionMode" class="library-manage-workspace">
              <aside class="library-manage-rail">
                <nav class="library-manage-group-list">
                  <button type="button" :class="{ active: !shelfGroup.shelfActiveGroup }"
                    :disabled="shelfBatch.batchBusy" @click.stop="shelfGroup.shelfActiveGroup = ''">
                    <span>全部内容</span><small>{{ groupBookCount('') }}</small>
                  </button>
                  <button v-for="group in shelfGroup.shelfGroups" :key="group" type="button"
                    :class="{ active: shelfGroup.shelfActiveGroup === group }"
                    :disabled="shelfBatch.batchBusy" @click.stop="shelfGroup.shelfActiveGroup = group">
                    <span>{{ group }}</span><small>{{ groupBookCount(group) }}</small>
                  </button>
                </nav>
                <button type="button" class="library-manage-groups-button"
:disabled="shelfBatch.batchBusy || mutationLocked"
                  @click.stop="manageGroupsOpen = !manageGroupsOpen">
                  {{ manageGroupsOpen ? '收起分组编辑' : '编辑分组' }}
                </button>
                <ShelfOrganizer v-if="manageGroupsOpen"
                  :groups="shelfGroup.shelfGroups" :active-group="shelfGroup.shelfActiveGroup"
                  :manage-groups-open="true"
                  @filter-group="shelfGroup.shelfActiveGroup = $event"
                  @create-group="shelfGroup.createGroup"
                  @rename-group="shelfGroup.renameGroup"
                  @delete-group="shelfGroup.removeShelfGroup"
                />
              </aside>
              <main class="library-manage-content">
                <div class="shelf-batch-toolbar library-manage-selectbar"
>
                  <label class="library-select-all">
                    <input type="checkbox" :checked="shelfBatch.allVisibleBooksSelected"
                      :disabled="shelfBatch.batchBusy || !shelfView.visibleBooks.length"
                      @change="shelfBatch.allVisibleBooksSelected ? shelfBatch.clearVisibleBookSelection() : shelfBatch.selectVisibleBooks()"/>
                    <span>全选</span>
                  </label>
                  <strong class="library-selection-count">已选 {{ shelfBatch.selectedVisibleBookIds.length }} 项</strong>
                  <div class="library-manage-primary-actions">
                    <button type="button" class="library-move-button"
                      :disabled="shelfBatch.batchBusy || mutationLocked || !shelfBatch.selectedVisibleBookIds.length || shelfBatch.selectedVisibleBookIds.length > 256"
                      @click="shelfBatch.openGroups">移动到</button>
                    <button type="button" class="library-remove-button"
                      :disabled="shelfBatch.batchBusy || mutationLocked || !shelfBatch.selectedVisibleBookIds.length || shelfBatch.selectedVisibleBookIds.length > 256"
                      @click="shelfBatch.openRemovalConfirm">移除</button>
                  </div>
                </div>
                <div v-if="shelfBatch.selectedVisibleBookIds.length > 256" class="shelf-batch-error">
                  每次最多处理 256 本书，请减少选择。
                </div>
                <p v-if="shelfBatch.batchError && !shelfBatch.groupsOpen" class="shelf-batch-error"
                  >{{ shelfBatch.batchError }}</p>
                <details class="library-manage-more">
                  <summary>更多批量操作</summary>
                  <div class="library-manage-extra-actions">
                    <button type="button"
                      :disabled="shelfBatch.batchBusy || !shelfBatch.selectedVisibleBookIds.length"
                      @click="shelfBatch.clearVisibleBookSelection">取消本组选择</button>
                    <button type="button"
                      :disabled="shelfBatch.batchBusy || mutationLocked || !shelfBatch.selectedVisibleBookIds.length || shelfBatch.selectedVisibleBookIds.length > 256"
                      @click="shelfBatch.startSelectedTasks('catalog')">更新目录</button>
                    <button type="button"
                      :disabled="shelfBatch.batchBusy || mutationLocked || !shelfBatch.selectedVisibleBookIds.length || shelfBatch.selectedVisibleBookIds.length > 256"
                      @click="shelfBatch.startSelectedTasks('download')">下载章节</button>
                    <label class="library-manage-sort-label">排序
                      <select :value="shelfView.shelfSortKey"
                        :disabled="shelfBatch.batchBusy || mutationLocked"
                        @change="shelfView.changeShelfSort(($event.target as HTMLSelectElement).value as ShelfSortKey)">
                        <option value="updatedAt">最近阅读</option><option value="title">书名</option>
                        <option value="author">作者</option><option value="latestChapter">最新章节</option>
                        <option value="progress">阅读进度</option><option value="custom">自定义顺序</option>
                      </select>
                    </label>
                    <button type="button" :disabled="shelfBatch.batchBusy || mutationLocked"
                      @click="shelfView.changeShelfSort(shelfView.shelfSortKey, shelfView.shelfSortOrder === 'ascending' ? 'descending' : 'ascending')">
                      {{ shelfView.shelfSortOrder === 'ascending' ? '升序' : '降序' }}
                    </button>
                    <span v-if="shelfBatch.selectedLocalBooks.length">所选包含 {{ shelfBatch.selectedLocalBooks.length }} 本本地书，不支持在线任务。</span>
                  </div>
                </details>
            <section v-if="shelfBatch.taskResult" class="shelf-batch-result-inline">
              <span>{{ shelfBatch.taskResult.operation === 'catalog' ? '目录更新' : '章节下载' }}任务：已启动 {{ shelfBatch.taskResult.started.length }} 本；本地书跳过 {{ shelfBatch.taskResult.skippedLocal.length }} 本；已有任务跳过 {{ shelfBatch.taskResult.skippedActive.length }} 本；当前未启动 {{ shelfBatch.taskResult.failed ? 1 : 0 }} 本；后续未启动 {{ shelfBatch.taskResult.notStarted.length }} 本。</span>
              <button class="text-button" @click="shelfBatch.taskResult = null">关闭结果</button>
              <div class="library-cover-control">
                <section v-if="shelfBatch.taskResult.blockedByRecovery" class="shelf-batch-result-section error">
                  <h3>恢复尚未完成</h3><p>本次未启动的任务不能在重启应用完成恢复前重试。</p>
                </section>
                <section v-if="shelfBatch.taskResult.started.length" class="shelf-batch-result-section">
                  <h3>已创建{{ shelfBatch.taskResult.operation === 'catalog' ? '目录更新' : '章节下载' }}任务</h3>
                  <ul><li v-for="book in shelfBatch.taskResult.started" :key="book.taskId"><strong>{{ book.title }}</strong><span>任务 {{ book.taskId }}</span></li></ul>
                </section>
                <section v-if="shelfBatch.taskResult.taskListWarnings.length" class="shelf-batch-result-section warning">
                  <h3>任务已创建，但列表刷新有提示</h3>
                  <ul><li v-for="warning in shelfBatch.taskResult.taskListWarnings" :key="warning.id"><strong>{{ warning.title }}</strong><span>{{ warning.message }}</span></li></ul>
                </section>
                <section v-if="shelfBatch.taskResult.failed" class="shelf-batch-result-section error">
                  <h3>{{ shelfBatch.taskResult.failed.outcome === 'unknown' ? '提交结果未确认' : '任务未启动' }}</h3>
                  <ul><li><strong>{{ shelfBatch.taskResult.failed.title }}</strong><span>{{ shelfBatch.taskResult.failed.message }}</span></li></ul>
                </section>
                <section v-if="shelfBatch.taskResult.notStarted.length" class="shelf-batch-result-section">
                  <h3>未启动</h3>
                  <ul><li v-for="book in shelfBatch.taskResult.notStarted" :key="book.id"><strong>{{ book.title }}</strong></li></ul>
                </section>
                <section v-if="shelfBatch.taskResult.skippedActive.length" class="shelf-batch-result-section">
                  <h3>已有{{ shelfBatch.taskResult.operation === 'catalog' ? '目录更新' : '章节下载' }}任务，未重复启动</h3>
                  <ul><li v-for="book in shelfBatch.taskResult.skippedActive" :key="book.id"><strong>{{ book.title }}</strong></li></ul>
                </section>
                <section v-if="shelfBatch.taskResult.skippedLocal.length" class="shelf-batch-result-section">
                  <h3>本地导入书不支持在线任务</h3>
                  <ul><li v-for="book in shelfBatch.taskResult.skippedLocal" :key="book.id"><strong>{{ book.title }}</strong></li></ul>
                </section>
              </div>
            </section>
            <div v-if="shelfBatch.removalResult && !shelfBatch.removalResultOpen" class="shelf-batch-result-inline">
              <span>批量移除：已提交 {{ shelfBatch.removalResult.completedIds.length }} 本，未提交 {{ shelfBatch.removalResult.errors.filter((entry) => entry.outcome === 'notCommitted').length }} 本，状态未知 {{ shelfBatch.removalResult.errors.filter((entry) => entry.outcome === 'unknown').length }} 本，未执行 {{ shelfBatch.removalResult.notAttemptedIds.length }} 本。</span>
              <button class="text-button" @click="shelfBatch.removalResultOpen = true">查看结果</button>
              <button v-if="!recoveryRequired" class="text-button" @click="shelfBatch.dismissRemovalSummary">关闭提示</button>
            </div>
                <div v-if="shelfView.visibleBooks.length" class="library-manage-grid">
                  <label v-for="book in shelfView.visibleBooks" :key="book.id"
                    class="library-manage-book" :class="{ selected: shelfBatch.selectedBookIds.includes(book.id) }"
                    >
                    <input type="checkbox"
                      :checked="shelfBatch.selectedBookIds.includes(book.id)" :disabled="shelfBatch.batchBusy"
@change="shelfBatch.toggleBookSelection(book.id)"/>
                    <span class="library-manage-cover" :class="`cover-tone-${Math.abs(book.title.length) % 5}`">
                      <img v-if="book.coverSrc" :src="book.coverSrc" loading="lazy" decoding="async"/>
                      <span v-else class="library-manage-cover-fallback"></span>
                      <span class="library-manage-tick">✓</span>
                    </span>
                    <strong>{{ book.title }}</strong><small>{{ book.author || '作者未知' }}</small>
                  </label>
                </div>
                <div v-else class="library-manage-empty">
                  <strong>{{ shelfView.shelf.books.length ? '这个分组还没有内容' : '内容库还没有书籍' }}</strong>
                  <p>{{ shelfView.shelf.books.length ? '选择其他分组或返回全部内容。' : '先从内容源添加书籍，或导入本地文件。' }}</p>
                  <button v-if="shelfView.shelf.books.length" type="button" @click="shelfGroup.shelfActiveGroup = ''">查看全部内容</button>
                  <button v-else type="button" @click="exitManageMode()">返回内容库</button>
                </div>
              </main>
            </div>
            <div v-if="!shelfBatch.selectionMode && displayedBooks.length" class="book-grid">
              <article v-for="book in displayedBooks" :key="book.id" class="shelf-card">
                <button class="cover-button" type="button" title="点击阅读，长按查看书籍信息" @pointerdown="shelfGroup.startShelfCoverLongPress(book.id, $event)" @pointermove="shelfGroup.trackShelfCoverLongPress" @pointerup="shelfGroup.finishShelfCoverLongPress" @pointercancel="shelfGroup.cancelShelfCoverLongPress" @pointerleave="shelfGroup.handleShelfCoverPointerLeave" @contextmenu.prevent @click="shelfGroup.handleShelfCoverClick(book.id)">
                  <img v-if="book.coverSrc" :src="book.coverSrc" class="book-cover" loading="lazy" />
                  <span v-else class="cover-fallback library-abstract-cover" :class="`cover-tone-${Math.abs(book.title.length) % 5}`"></span>
                  <span v-if="readingProgressPercent(book.progress, book.chapterCount) > 0" class="cover-progress-inset"><i :style="{ width: `${readingProgressPercent(book.progress, book.chapterCount)}%` }"></i></span>
                </button>
                <div class="shelf-card-info library-cardcopy">
                  <button class="book-title-button" @click="openBookInfo(book.id)">{{ book.title }}</button>
                  <span class="book-author">{{ book.author || '作者未知' }}</span>
                  <div class="book-card-meta"><span v-if="readingProgressPercent(book.progress, book.chapterCount)">{{ chapterCountLabel(book) }} · {{ readingProgressPercent(book.progress, book.chapterCount) }}%</span><span v-else>{{ chapterCountLabel(book) }}</span></div>
                </div>
              </article>
            </div>
            <div v-else-if="!shelfBatch.selectionMode && shelfView.shelf.books.length" class="library-filter-empty">
              <strong>当前筛选下没有内容</strong>
              <p>切换类型或分组，查看其他已收藏的书籍。</p>
              <button class="button secondary" @click.stop="activeType = 'all'; shelfGroup.shelfActiveGroup = ''">显示全部内容</button>
            </div>
            <div v-else-if="!shelfBatch.selectionMode" class="library-filter-empty library-empty">
              <strong>内容库还没有书籍</strong>
              <p>可从本地导入 TXT、EPUB、CBZ 或 PDF，或从书源中查找内容。</p>
              <div><button class="button secondary"
                :disabled="mutationLocked" @click.stop="openImport()">导入本地内容</button>
                <button class="button primary" @click.stop="openSearch()">搜索内容</button></div>
            </div>
          </section>
  </main>
</div>

    <section v-if="shelfBatch.removeConfirmOpen" class="modal-backdrop" @click.self="shelfBatch.cancelRemoval">
      <article class="app-modal shelf-batch-remove-modal">
        <button class="detail-close" :disabled="shelfBatch.batchBusy" @click="shelfBatch.cancelRemoval">×</button>
        <div class="confirm-symbol">⌫</div>
        <p class="eyebrow">批量移除</p>
        <h2 id="shelf-batch-remove-title">确认移除 {{ shelfBatch.removalTargets.length }} 本书？</h2>
        <p class="modal-description">将从书架中移除所选书籍、阅读进度、书签和本地缓存。请核对书名；取消不会修改书架。</p>
        <ul class="shelf-batch-remove-targets">
          <li v-for="book in shelfBatch.removalTargets" :key="book.id">
            <strong>{{ book.title }}</strong><span>{{ book.author || '作者未知' }}</span>
          </li>
        </ul>
        <p v-if="shelfBatch.removalError" class="shelf-batch-error">{{ shelfBatch.removalError }}</p>
        <div class="modal-actions">
          <button class="button secondary" :disabled="shelfBatch.batchBusy" @click="shelfBatch.cancelRemoval">取消</button>
          <button class="button danger" :disabled="shelfBatch.batchBusy || mutationLocked" @click="shelfBatch.submitRemoval">{{ shelfBatch.batchBusy ? '正在移除…' : `确认移除 ${shelfBatch.removalTargets.length} 本书` }}</button>
        </div>
      </article>
    </section>

    <section v-if="shelfBatch.removalResultOpen" class="modal-backdrop" @click.self="shelfBatch.closeRemovalResult">
      <article class="app-modal shelf-batch-result-modal">
        <button class="detail-close" @click="shelfBatch.closeRemovalResult">×</button>
        <p class="eyebrow">批量移除</p>
        <h2 id="shelf-batch-remove-result-title">处理结果</h2>
        <p v-if="shelfBatch.removalResult" class="modal-description">
          已提交移除 {{ shelfBatch.removalResult.completedIds.length }} 本；未提交 {{ shelfBatch.removalResult.errors.filter((entry) => entry.outcome === 'notCommitted').length }} 本；状态未知 {{ shelfBatch.removalResult.errors.filter((entry) => entry.outcome === 'unknown').length }} 本；未执行 {{ shelfBatch.removalResult.notAttemptedIds.length }} 本。
        </p>
        <p v-if="shelfBatch.removalError" class="shelf-batch-error">{{ shelfBatch.removalError }}</p>
        <p v-if="shelfBatch.shelfReadError" class="shelf-batch-error">{{ shelfBatch.shelfReadError }}</p>
        <div v-if="recoveryRequired" class="shelf-batch-recovery shelf-batch-result-recovery">
          <strong>{{ recoveryTitle }}</strong><span>{{ recoveryMessage }}</span>
        </div>
        <template v-if="shelfBatch.removalResult">
          <section v-if="shelfBatch.removalResult.completedIds.length" class="shelf-batch-result-section">
            <h3>已提交移除</h3>
            <ul><li v-for="id in shelfBatch.removalResult.completedIds" :key="id">{{ shelfBatch.removalTargetTitle(id) }}</li></ul>
          </section>
          <section v-if="shelfBatch.removalResult.warnings.length" class="shelf-batch-result-section warning">
            <h3>已移除，但有清理提示</h3>
            <ul><li v-for="warning in shelfBatch.removalResult.warnings" :key="warning.bookId"><strong>{{ shelfBatch.removalTargetTitle(warning.bookId) }}</strong><span>{{ warning.recoveryRequired ? '已提交，需重启恢复。' : '删除已提交。' }} {{ warning.message }}</span></li></ul>
          </section>
          <section v-if="shelfBatch.removalResult.errors.length" class="shelf-batch-result-section error">
            <h3>未完成或状态未知</h3>
            <ul><li v-for="entry in shelfBatch.removalResult.errors" :key="entry.bookId"><strong>{{ shelfBatch.removalTargetTitle(entry.bookId) }}</strong><span>{{ entry.outcome === 'notCommitted' ? '未提交，书籍未移除。' : '提交状态未知，请重启应用确认。' }} {{ entry.message }}</span></li></ul>
          </section>
          <section v-if="shelfBatch.removalResult.notAttemptedIds.length" class="shelf-batch-result-section">
            <h3>未执行</h3>
            <ul><li v-for="id in shelfBatch.removalResult.notAttemptedIds" :key="id">{{ shelfBatch.removalTargetTitle(id) }}<span>未尝试移除。</span></li></ul>
          </section>
        </template>
        <div class="modal-actions"><button class="button primary" @click="shelfBatch.closeRemovalResult">完成</button></div>
      </article>
    </section>

    <section v-if="shelfBatch.groupsOpen" class="modal-backdrop" @click.self="shelfBatch.closeGroups">
      <article class="app-modal shelf-batch-group-modal">
        <button class="detail-close" :disabled="shelfBatch.batchBusy" @click="shelfBatch.closeGroups">×</button>
        <p class="eyebrow">批量整理</p>
        <h2 id="shelf-batch-group-title">移动 {{ shelfBatch.groupBookIds.length }} 本内容至分组</h2>
        <p class="modal-description">所选分组将完整替换这些内容当前的分组，可同时指定多个分组。其他内容和空分组不受影响。</p>
        <div class="shelf-batch-group-list">
          <label v-for="group in shelfGroup.shelfGroups" :key="group" class="shelf-batch-group-option" :class="{ selected: shelfBatch.groupDraft.includes(group) }">
            <input type="checkbox" :checked="shelfBatch.groupDraft.includes(group)" :disabled="shelfBatch.batchBusy || mutationLocked || shelfBatch.clearGroups" @change="shelfBatch.toggleGroup(group)" />
            <span>{{ group }}</span>
          </label>
          <p v-if="!shelfGroup.shelfGroups.length" class="shelf-batch-groups-empty">还没有分组。可以在下方直接添加，或选择清空分组。</p>
        </div>
        <form class="shelf-batch-new-group" @submit.prevent="shelfBatch.addGroup">
          <label for="shelf-batch-new-group-input">添加并应用一个新分组</label>
          <div>
            <input id="shelf-batch-new-group-input" v-model="shelfBatch.newGroup" maxlength="128" autocomplete="off" placeholder="输入分组名称" :disabled="shelfBatch.batchBusy || mutationLocked" />
            <button class="button secondary small" type="submit" :disabled="shelfBatch.batchBusy || mutationLocked || !shelfBatch.newGroup.trim()">添加</button>
          </div>
        </form>
        <label class="shelf-batch-clear-option">
          <input type="checkbox" :checked="shelfBatch.clearGroups" :disabled="shelfBatch.batchBusy || mutationLocked" @change="shelfBatch.setClearGroups(($event.target as HTMLInputElement).checked)" />
          <span><strong>清空所选书的全部分组</strong><small>会把所选书籍的分组完整替换为空列表。</small></span>
        </label>
        <p v-if="shelfBatch.batchError" class="shelf-batch-error shelf-batch-modal-error">{{ shelfBatch.batchError }}</p>
        <div class="modal-actions">
          <button class="button secondary" :disabled="shelfBatch.batchBusy" @click="shelfBatch.closeGroups">取消</button>
          <button class="button primary" :disabled="shelfBatch.batchBusy || mutationLocked || (!shelfBatch.groupDraft.length && !shelfBatch.clearGroups)" @click="shelfBatch.submitGroups">{{ shelfBatch.batchBusy ? '正在保存…' : `确认移动 ${shelfBatch.groupBookIds.length} 项` }}</button>
        </div>
      </article>
    </section>
</template>
</template>

<style scoped>
.shelf-page-layout {
  display: flex;
  width: 100%;
  height: 100dvh;
  min-height: 0;
  overflow: hidden;
  padding-top: var(--safe-top);
  background: var(--app-background);
}
.shelf-page-main {
  flex: 1;
  min-width: 0;
  min-height: 0;
  overflow-x: hidden;
  overflow-y: auto;
  padding: 0 clamp(20px, 3.6vw, 56px) var(--safe-bottom);
}
.library-manage-page .shelf-page-main {
  padding: 0;
}
.shelf-view {
  width: 100%;
  max-width: 1120px;
  margin: 0 auto;
  padding: 16px 0 calc(24px + var(--safe-bottom));
}
.book-grid {
  display: grid;
  grid-template-columns: repeat(5, minmax(0, 1fr));
  gap: 22px 16px;
}
.cover-button {
  display: block;
  width: 100%;
  aspect-ratio: 4 / 5;
  overflow: hidden;
  padding: 0;
  border: 0;
  cursor: pointer;
}
.book-cover {
  display: block;
  width: 100%;
  height: 100%;
  object-fit: cover;
}
.book-cover,
.library-manage-cover img {
  -webkit-touch-callout: none;
  -webkit-user-drag: none;
  -webkit-user-select: none;
  user-select: none;
}
.cover-progress-inset {
  position: absolute;
  right: 8px;
  bottom: 8px;
  left: 8px;
  height: 4px;
}
.shelf-card-info {
  min-width: 0;
  padding-top: 8px;
}
.book-title-button, .book-author {
  display: block;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.book-card-meta {
  display: flex;
  gap: 7px;
  overflow: hidden;
  margin-top: 4px;
  font-size: 11px;
}
.library-cover-control {
  flex: 1 1 100%;
}
@media (max-width: 900px) {
  .book-grid {
    grid-template-columns: repeat(4, minmax(0, 1fr));
  }
}
@media (max-width: 720px) {
  .book-grid {
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 18px 10px;
  }
}
@media (max-width: 430px) {
  .book-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}
.library-topbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  margin: 8px 0 24px;
}
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
.library-type-filter button {
  flex: none;
  min-height: 31px;
  padding: 0 13px;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: #7a7f88;
  font-size: 12px;
  font-weight: 650;
  cursor: pointer;
}
.library-type-filter button.active {
  background: #202126;
  color: #fff;
}
.library-top-actions {
  display: flex;
  align-items: center;
  gap: 6px;
  flex: none;
}
.library-round-action {
  display: grid;
  place-items: center;
  width: 42px;
  height: 42px;
  border: 0;
  border-radius: 12px;
  background: #eceef2;
  color: #858b98;
  cursor: pointer;
}
.library-round-action:hover:not(:disabled) {
  color: var(--app-accent);
  background: #eae7f9;
}
.library-round-action:disabled {
  opacity: .5;
  cursor: default;
}
.library-round-action svg {
  width: 20px;
  height: 20px;
}
.library-cardcopy {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 3px;
}
.library-cardcopy .book-title-button {
  font-size: 13px;
  font-weight: 750;
}
.library-cardcopy .book-author {
  margin: 0;
  font-size: 10px;
}
.library-cardcopy .book-card-meta {
  margin: 0;
  font-size: 9px;
  color: #a0a4ac;
}
.library-cardcopy .book-card-meta span+span::before {
  display: none;
}
.library-abstract-cover {
  position: relative;
  display: block;
  overflow: hidden;
  padding: 0 !important;
  background: linear-gradient(135deg,#456c6a,#172e31) !important;
}
.library-abstract-cover::before {
  content: "";
  position: absolute;
  inset: 14% -15% auto auto;
  width: 70%;
  height: 70%;
  border-radius: 50%;
  background: rgba(255,255,255,.13);
}
.library-abstract-cover::after {
  content: "";
  position: absolute;
  left: -20%;
  right: -20%;
  bottom: -18%;
  height: 55%;
  border-radius: 50% 50% 0 0;
  background: rgba(0,0,0,.14);
  transform: rotate(-8deg);
}
.library-abstract-cover.cover-tone-1 {
  background: linear-gradient(135deg,#8a6d7f,#393246) !important;
}
.library-abstract-cover.cover-tone-2 {
  background: linear-gradient(135deg,#73939a,#2d4a45) !important;
}
.library-abstract-cover.cover-tone-3 {
  background: linear-gradient(135deg,#384a67,#121a28) !important;
}
.library-abstract-cover.cover-tone-4 {
  background: linear-gradient(135deg,#91677d,#362432) !important;
}
.library-filter-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 11px;
  min-height: 210px;
  padding: 36px 18px;
  text-align: center;
  color: #8c9099;
}
.library-filter-empty strong {
  color: #484a54;
  font-size: 15px;
}
.library-filter-empty p {
  margin: 0;
  font-size: 12px;
  line-height: 1.7;
}
.library-filter-empty>div {
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  gap: 8px;
  margin-top: 3px;
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
    font-size: 11px;
  }
  .library-top-actions {
    gap: 4px;
  }
  .library-round-action {
    width: 36px;
    height: 36px;
    border-radius: 11px;
  }
  .library-manage-sort {
    flex-wrap: wrap;
    gap: 8px;
  }
}
/*Dedicated management workspace, following prototype manage()/manage-layout.*/
.shelf-view.library-manage-mode {
  max-width: none;
  padding: 0 0 calc(48px + var(--safe-bottom));
  margin: 0;
  width: 100%;
}
.library-manage-heading {
  position: sticky;
  z-index: 4;
  top: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  min-height: 64px;
  padding: 0 max(18px,calc((100% - 1180px)/2));
  margin: 0;
  border-bottom: 1px solid #e7e8ef;
  background: #f6f7f9f5;
  backdrop-filter: blur(12px);
}
.library-manage-title {
  display: flex;
  align-items: center;
  gap: 12px;
}
.library-manage-title>span {
  display: flex;
  flex-direction: column;
  gap: 3px;
}
.library-manage-title strong {
  color: #31333c;
  font-size: 14px;
  font-weight: 750;
}
.library-manage-title small {
  color: #989ca7;
  font-size: 10px;
}
.library-done-button {
  min-height: 36px;
  padding: 0 14px;
  border: 0;
  border-radius: 10px;
  background: var(--app-accent);
  color: #fff;
  font-size: 11px;
  font-weight: 750;
  cursor: pointer;
}
.library-done-button:disabled {
  opacity: .5;
  cursor: default;
}
.library-manage-workspace {
  display: grid;
  grid-template-columns: 190px minmax(0,1fr);
  gap: 28px;
  align-items: start;
  width: min(1120px,calc(100% - 36px));
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
  border: 0;
  border-radius: 9px;
  background: transparent;
  color: #777d8a;
  font-size: 11px;
  text-align: left;
  cursor: pointer;
}
.library-manage-group-list button.active {
  background: #ece9fb;
  color: #5f53ad;
}
.library-manage-group-list button:hover:not(.active) {
  background: #eaeaf0;
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
  font-size: 10px;
  opacity: .75;
}
.library-manage-groups-button {
  align-self: flex-start;
  min-height: 28px;
  padding: 0 9px;
  border: 0;
  border-radius: 8px;
  background: #f0edfa;
  color: var(--app-accent);
  font-size: 10px;
  cursor: pointer;
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
  border: 1px solid #e4e1ef;
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
.library-manage-rail :deep(.new-group-form input) {
  width: 100%;
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
  border-bottom: 1px solid #e6e8ee;
  border-radius: 0;
  padding: 0 3px 8px;
  background: transparent;
  box-shadow: none;
}
.library-select-all {
  display: flex;
  align-items: center;
  gap: 7px;
  color: #646a74;
  font-size: 11px;
  white-space: nowrap;
  cursor: pointer;
}
.library-select-all input {
  width: 15px;
  height: 15px;
  margin: 0;
  accent-color: var(--app-accent);
  cursor: pointer;
}
.library-selection-count {
  margin-right: auto;
  color: #999da6;
  font-size: 10px;
  font-weight: 500;
  white-space: nowrap;
}
.library-manage-primary-actions {
  display: flex;
  align-items: center;
  gap: 7px;
}
.library-move-button,.library-remove-button {
  min-height: 35px;
  padding: 0 12px;
  border: 0;
  border-radius: 10px;
  font-size: 11px;
  font-weight: 700;
  cursor: pointer;
}
.library-move-button {
  background: #eceef2;
  color: #636976;
}
.library-remove-button {
  background: #fff0f1;
  color: #b4535f;
}
.library-move-button:hover:not(:disabled) {
  background: #e5e0fa;
  color: var(--app-accent);
}
.library-remove-button:hover:not(:disabled) {
  background: #ffe4e8;
}
.library-move-button:disabled,.library-remove-button:disabled {
  opacity: .45;
  cursor: default;
}
.library-manage-grid {
  display: grid;
  grid-template-columns: repeat(4,minmax(0,1fr));
  gap: 17px 14px;
  padding-top: 16px;
}
.library-manage-book {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
  cursor: pointer;
}
.library-manage-book>input {
  position: absolute;
  z-index: 2;
  left: 9px;
  top: 9px;
  width: 17px;
  height: 17px;
  margin: 0;
  accent-color: var(--app-accent);
  cursor: pointer;
}
.library-manage-cover {
  display: block;
  position: relative;
  overflow: hidden;
  aspect-ratio: 4/5;
  border-radius: 10px;
  background: linear-gradient(145deg,#406b68,#172e31);
}
.library-manage-cover img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}
.library-manage-cover-fallback {
  position: absolute;
  inset: 0;
}
.library-manage-cover-fallback:before {
  content: "";
  position: absolute;
  right: -25%;
  top: 14%;
  width: 72%;
  height: 72%;
  border-radius: 50%;
  background: #fff2;
}
.library-manage-cover-fallback:after {
  content: "";
  position: absolute;
  left: -20%;
  right: -18%;
  bottom: -22%;
  height: 53%;
  border-radius: 50%;
  background: #0003;
}
.library-manage-cover.cover-tone-1 {
  background: linear-gradient(145deg,#8a6d7f,#393246);
}
.library-manage-cover.cover-tone-2 {
  background: linear-gradient(145deg,#73939a,#2d4a45);
}
.library-manage-cover.cover-tone-3 {
  background: linear-gradient(145deg,#384a67,#121a28);
}
.library-manage-cover.cover-tone-4 {
  background: linear-gradient(145deg,#91677d,#362432);
}
.library-manage-tick {
  position: absolute;
  display: none;
  right: 7px;
  top: 7px;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  border-radius: 50%;
  background: var(--app-accent);
  color: white;
}
.library-manage-tick {
  font-size: 14px;
  font-weight: 800;
}
.library-manage-book.selected .library-manage-tick {
  display: flex;
}
.library-manage-book.selected .library-manage-cover {
  outline: 2px solid var(--app-accent);
  outline-offset: 2px;
}
.library-manage-book>strong,.library-manage-book>small {
  display: block;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.library-manage-book>strong {
  margin-top: 2px;
  color: #34363e;
  font-size: 11px;
}
.library-manage-book>small {
  color: #9da1aa;
  font-size: 9px;
}
.library-manage-more {
  margin-top: 9px;
  color: #7e8490;
  font-size: 11px;
}
.library-manage-more summary {
  display: inline-block;
  padding: 6px 1px;
  cursor: pointer;
}
.library-manage-more summary::marker {
  color: #7d75be;
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
  border: 1px solid #e1e2ea;
  border-radius: 8px;
  background: #fff;
  color: #656b78;
  cursor: pointer;
}
.library-manage-extra-actions button:disabled {
  opacity: .5;
  cursor: default;
}
.library-manage-extra-actions span {
  color: #a08164;
  font-size: 10px;
}
.library-manage-sort-label {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 10px;
}
.library-manage-sort-label select {
  height: 30px;
  min-width: 110px;
  padding: 0 7px;
  border: 1px solid #e1e2ea;
  border-radius: 8px;
  background: #fff;
  color: #646a77;
  font-size: 10px;
}
.library-manage-empty {
  display: flex;
  flex-direction: column;
  justify-content: center;
  align-items: center;
  gap: 10px;
  min-height: 220px;
  padding: 18px;
  color: #8e929d;
  text-align: center;
}
.library-manage-empty strong {
  color: #555763;
  font-size: 13px;
}
.library-manage-empty p {
  margin: 0;
  font-size: 11px;
}
.library-manage-empty button {
  padding: 8px 12px;
  border: 0;
  border-radius: 9px;
  color: #6254bc;
  background: #efebff;
  cursor: pointer;
}
@media (max-width:860px) {
  .shelf-page-main {
    padding: 0 14px calc(67px + var(--safe-bottom));
  }
  .library-manage-page .shelf-page-main {
    padding: 0;
  }
  .library-manage-heading {
    min-height: 54px;
    padding: 0 12px;
  }
  .library-manage-workspace {
    display: flex;
    flex-direction: column;
    gap: 0;
    margin: 0;
    width: 100%;
  }
  .library-manage-rail {
    display: block;
    width: 100%;
    padding: 12px;
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
    border-radius: 999px;
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
    padding: 0 12px 35px;
  }
  .library-manage-selectbar {
    min-height: 47px;
  }
  .library-manage-grid {
    grid-template-columns: repeat(3,minmax(0,1fr));
  }
}
@media (max-width:480px) {
  .library-manage-grid {
    grid-template-columns: repeat(2,minmax(0,1fr));
    gap: 16px 11px;
  }
  .library-manage-selectbar {
    gap: 7px;
  }
  .library-manage-primary-actions {
    gap: 4px;
  }
  .library-move-button,.library-remove-button {
    min-height: 32px;
    padding: 0 9px;
  }
}
</style>

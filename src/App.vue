<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import BookGroupsEditor from "./components/BookGroupsEditor.vue";
import ReadingInsights from "./components/ReadingInsights.vue";
import ShelfOrganizer from "./components/ShelfOrganizer.vue";
import TaskCenter from "./components/TaskCenter.vue";
import BackupControls from "./components/BackupControls.vue";
import {
  addBook,
  appBootstrap,
  cancelTask,
  checkNewChapters,
  clearReadingHistory,
  deleteBookmark,
  deleteReadingHistoryForBook,
  deleteReplacementRule,
  deleteShelfGroup,
  createBackupFromPicker,
  createShelfGroup,
  getBook,
  importBookFromPicker,
  importSourcesFromPicker,
  listDiscoveryBooks,
  listDiscoveryCategories,
  listRssArticles,
  listRssCategories,
  listSources,
  openRssArticle,
  pauseTask,
  prepareChapters,
  readResource,
  readTextResource,
  readingHistoryResource,
  recordReadingSession,
  renameShelfGroup,
  removeBook,
  removeSources,
  refreshChapters,
  restoreBackupFromPicker,
  resumeTask,
  saveProgress,
  saveSettings,
  setBookGroups,
  setShelfSort,
  startChapterDownload,
  startSearch,
  tasksResource,
  listBookmarks,
  listReplacementRules,
  upsertBookmark,
  upsertReplacementRule,
  type AppBootstrap,
  type AppSettingsResource,
  type AppTask,
  type BackupResponse,
  type Bookmark,
  type BookmarksResource,
  type BookResource,
  type DiscoveryCategoriesResource,
  type DiscoveryCategory,
  type DisplayReplacementRule,
  type ReadingHistoryResource,
  type ReadingProgress,
  type ReaderSettings,
  type ResourceDescriptor,
  type SearchBookResult,
  type SearchResource,
  type ShelfResource,
  type ShelfSortKey,
  type ShelfSortOrder,
  type TasksResource,
  type SourcePatch,
  type SourceMetadata,
  ResourceHttpError,
  updateSource,
} from "./api/app";

type Screen = "home" | "shelf" | "search" | "sources" | "settings";
type ReaderTheme = ReaderSettings["theme"];
type DiscoverMode = "discover" | "search" | "rss";

const defaultSettings: AppSettingsResource = {
  schemaVersion: 1,
  reader: {
    fontSizePx: 19,
    lineHeight: 1.8,
    fontFamily: "serif",
    textColor: "#3f3b34",
    backgroundColor: "#f7f3e9",
    textAlign: "justify",
    theme: "paper",
    preloadCount: 5,
    replacements: [],
  },
};

const screen = ref<Screen>("home");
const bootstrapped = ref(false);
const isLoading = ref(false);
const globalError = ref("");
const toast = ref("");
const toastKind = ref<"success" | "error">("success");
const shelf = ref<ShelfResource>({ schemaVersion: 1, books: [] });
const sources = ref<SourceMetadata[]>([]);
const settings = ref<AppSettingsResource>(structuredClone(defaultSettings));
const tasks = ref<AppTask[]>([]);
const discoverMode = ref<DiscoverMode>("discover");
const discoverySourceId = ref("");
const discoveryCategories = ref<DiscoveryCategory[]>([]);
const discoveryCategoryId = ref("");
const discoveryResults = ref<SearchBookResult[]>([]);
const discoveryResourceErrors = ref<Array<{ sourceId?: string; message: string }>>([]);
const discoveryPage = ref(1);
const discoveryHasNextPage = ref(false);
const discoveryBusy = ref(false);
const discoveryWasLoaded = ref(false);
const discoveryError = ref("");
const discoverySearchResources = new Map<string, ResourceDescriptor>();
const activeSearchTaskId = ref<string | null>(null);
const articleOpen = ref(false);
const articleBusy = ref(false);
const articleTitle = ref("");
const articleHtml = ref("");
const bookmarks = ref<Bookmark[]>([]);
const readingHistory = ref<ReadingHistoryResource>({ schemaVersion: 1, sessions: [], books: [], days: [], totalDurationMs: 0, totalSessions: 0 });
const replacementRules = ref<DisplayReplacementRule[]>([]);
const shelfQuery = ref("");
const shelfActiveGroup = ref("");
const selectedSourceIds = ref<string[]>([]);
const searchKeyword = ref("");
const searchPage = ref(1);
const searchBusy = ref(false);
const searchProgress = ref("");
const searchResponseErrors = ref<Array<{ sourceId?: string; message: string }>>([]);
const searchResults = ref<SearchBookResult[]>([]);
const searchResourceErrors = ref<Array<{ sourceId?: string; message: string }>>([]);
const searchWasRun = ref(false);
const selectedResult = ref<SearchBookResult | null>(null);
const openedBook = ref<BookResource | null>(null);
const bookPanelBusy = ref(false);
const addBusyResult = ref<string | null>(null);
const pendingRemoval = ref<{ id: string; title: string } | null>(null);
const pendingSourceRemoval = ref<string[]>([]);
const sourceQuery = ref("");
const sourceImportOpen = ref(false);
const sourceImportBusy = ref(false);
const localBookImportBusy = ref(false);
const selectedSourceRows = ref<string[]>([]);
const sourceFilter = ref<"all" | "enabled" | "disabled">("all");
const sourceGroupFilter = ref("");
const editingSource = ref<SourceMetadata | null>(null);
const sourceEditName = ref("");
const sourceEditGroup = ref("");
const bookmarkEditorOpen = ref(false);
const bookmarkNote = ref("");
const bookmarkBusy = ref(false);
const readingSessionId = ref<string | null>(null);
const readingSessionStartedAt = ref<number | null>(null);
const groupBusy = ref(false);
const backupBusy = ref<"create" | "restore" | null>(null);
const lastBackupAt = ref<number | null>(null);

const readerVisible = ref(false);
const readingBook = ref<BookResource | null>(null);
const readingChapterIndex = ref(0);
const readingChapterHtml = ref("");
const readingChapterRaw = ref("");
const readerFrame = ref<HTMLIFrameElement | null>(null);
const readerPageIndex = ref(0);
const readerPageCount = ref(1);
const readerControlsOpen = ref(false);
const readerBusy = ref(false);
const readerProgressDirty = ref(false);
const progressSaveState = ref<"idle" | "saving" | "saved" | "error">("idle");
const prefetchBusy = ref(false);
const saveSettingsBusy = ref(false);
const pendingSettingsSave = ref<ReturnType<typeof setTimeout> | null>(null);
const unlisteners: UnlistenFn[] = [];
let toastTimer: ReturnType<typeof setTimeout> | undefined;
let progressSavePromise: Promise<void> | null = null;
let preparePromise: Promise<void> | null = null;
let progressRevision = 0;
let replacementSaveTimer: ReturnType<typeof setTimeout> | null = null;
const replacementWrites = new Map<string, Promise<void>>();
function handleReaderResize(): void {
  measureReaderPages(true);
}

const shelfBooks = computed(() => {
  const query = shelfQuery.value.trim().toLocaleLowerCase();
  const books = shelf.value.books ?? [];
  return books.filter((book) => {
    const matchesQuery = !query || `${book.title} ${book.author ?? ""}`.toLocaleLowerCase().includes(query);
    const matchesGroup = !shelfActiveGroup.value || (book.groups ?? []).includes(shelfActiveGroup.value);
    return matchesQuery && matchesGroup;
  });
});
const shelfGroups = computed(() => [...new Set([...(shelf.value.groups ?? []), ...(shelf.value.books ?? []).flatMap((book) => book.groups ?? [])])].sort((a, b) => a.localeCompare(b, "zh-CN")));
const shelfSortKey = computed(() => shelf.value.sort ?? "updatedAt");
const shelfSortOrder = computed(() => shelf.value.sortOrder ?? "descending");
const bookTitleMap = computed(() => Object.fromEntries((shelf.value.books ?? []).map((book) => [book.id, book.title])));
const recentBooks = computed(() => [...(shelf.value.books ?? [])].filter((book) => book.progress?.updatedAtMs).sort((a, b) => (b.progress?.updatedAtMs ?? 0) - (a.progress?.updatedAtMs ?? 0)).slice(0, 6));
const enabledSources = computed(() => sources.value.filter((source) => source.enabled !== false));
const sourceGroups = computed(() => [...new Set(sources.value.map((source) => source.group).filter((group): group is string => Boolean(group)))].sort());
const filteredSources = computed(() => {
  const query = sourceQuery.value.trim().toLocaleLowerCase();
  return sources.value.filter((source) => {
    const matchesText = !query || `${source.name} ${source.group ?? ""} ${source.id}`.toLocaleLowerCase().includes(query);
    const matchesStatus = sourceFilter.value === "all" || (sourceFilter.value === "enabled" ? source.enabled : !source.enabled);
    const matchesGroup = !sourceGroupFilter.value || source.group === sourceGroupFilter.value;
    return matchesText && matchesStatus && matchesGroup;
  });
});
const currentChapter = computed(() => readingBook.value?.chapters?.find((chapter) => chapter.index === readingChapterIndex.value) ?? null);
const currentChapterTitle = computed(() => currentChapter.value?.title ?? `第 ${readingChapterIndex.value + 1} 章`);
const readerSettings = computed(() => settings.value.reader);
const currentBookmark = computed(() => bookmarks.value.find((bookmark) => bookmark.bookId === readingBook.value?.id && bookmark.chapterIndex === readingChapterIndex.value && bookmark.offset === readerPageIndex.value) ?? null);
const readPercent = computed(() => {
  const total = Math.max(1, readingBook.value?.chapterCount ?? readingBook.value?.chapters.length ?? 1);
  return Math.min(100, Math.round(((readingChapterIndex.value + 1) / total) * 100));
});
const selectedSearchSources = computed(() => enabledSources.value.length > 0 && selectedSourceIds.value.length === enabledSources.value.length);

function notify(message: string, kind: "success" | "error" = "success"): void {
  toast.value = message;
  toastKind.value = kind;
  if (toastTimer) clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (toast.value = ""), 3200);
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

async function refreshShelfAndSources(): Promise<void> {
  const bootstrap: AppBootstrap = await appBootstrap();
  const [nextShelf, nextSettings] = await Promise.all([
    readResource<ShelfResource>(bootstrap.shelf),
    readResource<AppSettingsResource>(bootstrap.settings),
  ]);
  shelf.value = { ...nextShelf, groups: Array.isArray(nextShelf.groups) ? nextShelf.groups : [], books: Array.isArray(nextShelf.books) ? nextShelf.books : [] };
  settings.value = normalizeSettings(nextSettings);
  sources.value = Array.isArray(bootstrap.sources) ? bootstrap.sources : [];
  selectedSourceIds.value = enabledSources.value.map((source) => source.id);
  bootstrapped.value = true;
  void refreshReadingData().catch((error) => notify(`无法读取阅读记录：${errorText(error)}`, "error"));
  void refreshTasks().catch((error) => notify(`无法读取任务：${errorText(error)}`, "error"));
}

async function refreshReadingData(): Promise<void> {
  const [bookmarksDescriptor, historyDescriptor, replacementDescriptor] = await Promise.all([
    listBookmarks(),
    readingHistoryResource(),
    listReplacementRules(),
  ]);
  const [nextBookmarks, nextHistory, nextRules] = await Promise.all([
    readResource<BookmarksResource>(bookmarksDescriptor),
    readResource<ReadingHistoryResource>(historyDescriptor),
    readResource<{ schemaVersion: number; rules: DisplayReplacementRule[] }>(replacementDescriptor),
  ]);
  bookmarks.value = Array.isArray(nextBookmarks.bookmarks) ? nextBookmarks.bookmarks : [];
  readingHistory.value = {
    schemaVersion: nextHistory.schemaVersion ?? 1,
    sessions: Array.isArray(nextHistory.sessions) ? nextHistory.sessions : [],
    books: Array.isArray(nextHistory.books) ? nextHistory.books : [],
    days: Array.isArray(nextHistory.days) ? nextHistory.days : [],
    totalDurationMs: nextHistory.totalDurationMs ?? 0,
    totalSessions: nextHistory.totalSessions ?? 0,
  };
  replacementRules.value = Array.isArray(nextRules.rules) ? nextRules.rules : [];
}

async function refreshShelfFromDescriptor(descriptor: ResourceDescriptor): Promise<void> {
  const next = await readResource<ShelfResource>(descriptor);
  shelf.value = { ...next, groups: Array.isArray(next.groups) ? next.groups : [], books: Array.isArray(next.books) ? next.books : [] };
}

async function changeShelfSort(key: ShelfSortKey, order: ShelfSortOrder = shelfSortOrder.value): Promise<void> {
  try {
    await refreshShelfFromDescriptor(await setShelfSort(key, order));
  } catch (error) {
    notify(`书架排序失败：${errorText(error)}`, "error");
  }
}

async function createGroup(groupName: string): Promise<void> {
  const normalized = groupName.trim();
  if (!normalized) return;
  if (shelfGroups.value.includes(normalized)) {
    notify("这个分组已经存在。", "error");
    return;
  }
  try {
    await refreshShelfFromDescriptor(await createShelfGroup(normalized));
    notify("书架分组已创建。已创建的空分组会保留。 ");
  } catch (error) {
    notify(`创建分组失败：${errorText(error)}`, "error");
  }
}

async function refreshTasks(descriptor?: ResourceDescriptor): Promise<void> {
  const resource = descriptor ?? (await tasksResource()).resource;
  const document = await readResource<TasksResource>(resource);
  tasks.value = Array.isArray(document.tasks) ? document.tasks : [];
  await syncActiveSearchTask();
}

const completedSearchTasks = new Set<string>();

async function syncActiveSearchTask(task?: Partial<AppTask>): Promise<void> {
  const taskId = activeSearchTaskId.value;
  if (!taskId) return;
  const current = task?.id === taskId ? task : tasks.value.find((entry) => entry.id === taskId);
  if (!current) return;
  if (current.status === "queued" || current.status === "running" || current.status === "pausing" || current.status === "paused") {
    searchBusy.value = current.status !== "paused";
    searchProgress.value = current.status === "paused" ? "搜索已暂停。" : `正在搜索书源（${current.completed ?? 0}/${current.total ?? 0}）…`;
    return;
  }
  if (completedSearchTasks.has(taskId)) return;
  completedSearchTasks.add(taskId);
  searchBusy.value = false;
  if (current.status === "completed") {
    const descriptor = discoverySearchResources.get(taskId);
    if (!descriptor) {
      searchProgress.value = "搜索已完成，但结果资源暂不可用。";
      notify(searchProgress.value, "error");
    } else {
      try {
        await loadSearchResource(descriptor);
        searchProgress.value = `搜索完成 · ${searchResults.value.length} 本书`;
      } catch (error) {
        searchProgress.value = "搜索完成，结果读取失败。";
        notify(`读取搜索结果失败：${errorText(error)}`, "error");
      }
    }
  } else if (current.status === "failed") {
    searchProgress.value = "搜索失败";
    notify(`搜索失败：${current.error ?? "请稍后重试。"}`, "error");
  } else if (current.status === "cancelled") {
    searchProgress.value = "搜索已取消。";
  }
  activeSearchTaskId.value = null;
}

async function renameGroup(oldName: string, newName: string): Promise<void> {
  if (!newName.trim()) return;
  try {
    await refreshShelfFromDescriptor(await renameShelfGroup(oldName, newName.trim()));
    if (shelfActiveGroup.value === oldName) shelfActiveGroup.value = newName.trim();
    notify("分组名称已更新。");
  } catch (error) {
    notify(`分组重命名失败：${errorText(error)}`, "error");
  }
}

async function removeShelfGroup(groupName: string): Promise<void> {
  try {
    await refreshShelfFromDescriptor(await deleteShelfGroup(groupName));
    if (shelfActiveGroup.value === groupName) shelfActiveGroup.value = "";
    notify("分组已删除，书籍仍保留在书架。");
  } catch (error) {
    notify(`删除分组失败：${errorText(error)}`, "error");
  }
}

async function saveBookGroups(bookId: string, groups: string[]): Promise<void> {
  if (groupBusy.value) return;
  groupBusy.value = true;
  try {
    await refreshShelfFromDescriptor(await setBookGroups(bookId, groups));
    notify("书籍分组已更新。");
  } catch (error) {
    notify(`更新书籍分组失败：${errorText(error)}`, "error");
  } finally {
    groupBusy.value = false;
  }
}

function normalizeSettings(value: AppSettingsResource): AppSettingsResource {
  const reader = value?.reader ?? {};
  const storedTheme = typeof reader.theme === "string" ? reader.theme : defaultSettings.reader.theme;
  const theme: ReaderTheme = storedTheme === "light"
    ? "system"
    : (["paper", "sepia", "dark", "system"].includes(storedTheme) ? storedTheme as ReaderTheme : "paper");
  const fontSize = Number(reader.fontSizePx ?? (reader as Partial<ReaderSettings> & { fontSize?: number }).fontSize ?? defaultSettings.reader.fontSizePx);
  const lineHeight = Number(reader.lineHeight ?? defaultSettings.reader.lineHeight);
  const preloadCount = Number(reader.preloadCount ?? defaultSettings.reader.preloadCount);
  return {
    ...defaultSettings,
    ...value,
    reader: {
      ...defaultSettings.reader,
      ...reader,
      fontSizePx: clamp(fontSize, 12, 36),
      lineHeight: clamp(lineHeight, 1.2, 2.8),
      preloadCount: clamp(preloadCount, 1, 20),
      theme,
      replacements: [],
    },
  };
}

async function bootstrap(): Promise<void> {
  isLoading.value = true;
  globalError.value = "";
  try {
    await refreshShelfAndSources();
  } catch (error) {
    globalError.value = errorText(error);
  } finally {
    isLoading.value = false;
  }
}

async function refreshSources(): Promise<void> {
  const response = await listSources();
  sources.value = Array.isArray(response.sources) ? response.sources : [];
  selectedSourceIds.value = enabledSources.value.map((source) => source.id);
}

async function changeSourceEnabled(source: SourceMetadata, enabled: boolean): Promise<void> {
  try {
    const response = await updateSource(source.id, { enabled });
    sources.value = response.sources;
    if (!enabled) selectedSourceIds.value = selectedSourceIds.value.filter((id) => id !== source.id);
    else if (!selectedSourceIds.value.includes(source.id)) selectedSourceIds.value.push(source.id);
    notify(`${source.name} 已${enabled ? "启用" : "停用"}。`);
  } catch (error) {
    notify(`更新书源状态失败：${errorText(error)}`, "error");
    await refreshSources().catch((refreshError) => notify(errorText(refreshError), "error"));
  }
}

function beginSourceEdit(source: SourceMetadata): void {
  editingSource.value = source;
  sourceEditName.value = source.name;
  sourceEditGroup.value = source.group ?? "";
}

async function saveSourceMetadata(): Promise<void> {
  const source = editingSource.value;
  if (!source) return;
  const name = sourceEditName.value.trim();
  if (!name) {
    notify("书源名称不能为空。", "error");
    return;
  }
  try {
    const patch: SourcePatch = { name, group: sourceEditGroup.value.trim() || null };
    const response = await updateSource(source.id, patch);
    sources.value = response.sources;
    editingSource.value = null;
    notify("书源名称和分组已更新。");
  } catch (error) {
    notify(`更新书源信息失败：${errorText(error)}`, "error");
  }
}

function chooseScreen(next: Screen): void {
  if (readerVisible.value && next !== screen.value) void closeReader();
  screen.value = next;
  globalError.value = "";
  openedBook.value = null;
  selectedResult.value = null;
  if (next === "sources") void refreshSources().catch((error) => notify(errorText(error), "error"));
  if (next === "search") void loadDiscoveryCategoriesAfterNav();
}

function formatReadingDuration(value: number): string {
  const minutes = Math.floor(Math.max(0, value) / 60_000);
  if (minutes < 60) return `${minutes} 分钟`;
  const hours = Math.floor(minutes / 60);
  return `${hours} 小时 ${minutes % 60} 分钟`;
}

async function continueReading(bookId: string): Promise<void> {
  try {
    const descriptor = await getBook(bookId);
    const book = await readResource<BookResource>(descriptor);
    await startReading(book);
  } catch (error) {
    notify(`打开阅读位置失败：${errorText(error)}`, "error");
  }
}

function toggleCurrentBookmark(): void {
  const existing = currentBookmark.value;
  if (existing) {
    void removeBookmark(existing.id);
  } else {
    bookmarkNote.value = "";
    bookmarkEditorOpen.value = true;
  }
}

async function loadSearchResource(descriptor: ResourceDescriptor): Promise<void> {
  const result = await readResource<SearchResource>(descriptor);
  searchResults.value = Array.isArray(result.results) ? result.results : [];
  searchResourceErrors.value = Array.isArray(result.errors) ? result.errors : [];
}

async function runSearch(page = 1): Promise<void> {
  if (!searchKeyword.value.trim()) {
    notify("先输入书名、作者或关键词。", "error");
    return;
  }
  if (!selectedSourceIds.value.length) {
    notify("请至少选择一个已启用书源。", "error");
    return;
  }
  searchBusy.value = true;
  searchWasRun.value = true;
  searchPage.value = page;
  searchProgress.value = "正在准备搜索…";
  searchResponseErrors.value = [];
  searchResourceErrors.value = [];
  searchResults.value = [];
  try {
    const response = await startSearch(selectedSourceIds.value, searchKeyword.value.trim(), page);
    if (!response.taskId) throw new Error("搜索任务没有返回编号。");
    discoverySearchResources.set(response.taskId, response.resource);
    activeSearchTaskId.value = response.taskId;
    searchProgress.value = "已开始搜索所选书源。";
    await refreshTasks().catch(() => {});
  } catch (error) {
    searchProgress.value = "搜索失败";
    notify(`搜索失败：${errorText(error)}`, "error");
    searchBusy.value = false;
  }
}

async function controlTask(action: "pause" | "resume" | "cancel", taskId: string): Promise<void> {
  try {
    const response = action === "pause" ? await pauseTask(taskId) : action === "resume" ? await resumeTask(taskId) : await cancelTask(taskId);
    await refreshTasks(response.resource);
  } catch (error) {
    notify(`更新任务失败：${errorText(error)}`, "error");
    await refreshTasks().catch(() => {});
  }
}

async function enqueueBookTask(action: "download" | "refresh" | "check"): Promise<void> {
  const book = openedBook.value;
  if (!book) return;
  try {
    let response;
    if (action === "download") {
      const fromIndex = book.chapters.find((chapter) => !chapter.src)?.index ?? 0;
      response = await startChapterDownload(book.id, fromIndex, Math.max(1, settings.value.reader.preloadCount));
    } else if (action === "refresh") {
      response = await refreshChapters(book.id);
    } else {
      response = await checkNewChapters(book.id);
    }
    await refreshTasks(response.resource);
    notify(action === "download" ? "章节准备任务已加入。" : action === "refresh" ? "目录更新任务已加入。" : "新章节检查已加入。 ");
  } catch (error) {
    notify(`无法创建任务：${errorText(error)}`, "error");
  }
}

async function loadDiscoveryCategories(): Promise<void> {
  if (!discoverySourceId.value) {
    discoveryCategories.value = [];
    discoveryCategoryId.value = "";
    discoveryResults.value = [];
    discoveryError.value = "请先选择一个已启用的书源。";
    return;
  }
  discoveryBusy.value = true;
  discoveryError.value = "";
  discoveryCategories.value = [];
  discoveryResults.value = [];
  discoveryCategoryId.value = "";
  discoveryWasLoaded.value = true;
  try {
    const response = discoverMode.value === "rss"
      ? await listRssCategories(discoverySourceId.value)
      : await listDiscoveryCategories(discoverySourceId.value);
    const document = await readResource<DiscoveryCategoriesResource>(response.resource);
    discoveryCategories.value = Array.isArray(document.categories) ? document.categories : [];
    const first = discoveryCategories.value.find((category) => category.categoryId);
    if (first?.categoryId) {
      discoveryCategoryId.value = first.categoryId;
      await loadDiscoveryPage(first.categoryId, 1);
    } else if (!discoveryCategories.value.length) {
      discoveryError.value = "这个书源暂时没有可浏览的分类。";
    }
  } catch (error) {
    discoveryError.value = errorText(error);
  } finally {
    discoveryBusy.value = false;
  }
}

async function loadDiscoveryPage(categoryId = discoveryCategoryId.value, page = 1): Promise<void> {
  if (!discoverySourceId.value || !categoryId) return;
  discoveryBusy.value = true;
  discoveryError.value = "";
  discoveryPage.value = page;
  try {
    const response = discoverMode.value === "rss"
      ? await listRssArticles(discoverySourceId.value, categoryId, page)
      : await listDiscoveryBooks(discoverySourceId.value, categoryId, page);
    const document = await readResource<SearchResource>(response.resource);
    discoveryResults.value = Array.isArray(document.results) ? document.results : [];
    discoveryHasNextPage.value = Boolean(response.hasNextPage);
    discoveryResourceErrors.value = Array.isArray(document.errors) ? document.errors : [];
    if (!discoveryResults.value.length && !discoveryResourceErrors.value.length) {
      discoveryError.value = discoverMode.value === "rss" ? "这个分类还没有文章。" : "这个分类暂时没有书籍。";
    }
  } catch (error) {
    discoveryError.value = errorText(error);
  } finally {
    discoveryBusy.value = false;
  }
}

function setDiscoverMode(mode: DiscoverMode): void {
  if (discoverMode.value === mode) return;
  discoverMode.value = mode;
  discoveryCategories.value = [];
  discoveryCategoryId.value = "";
  discoveryResults.value = [];
  discoveryWasLoaded.value = false;
  discoveryError.value = "";
  if (mode !== "search" && discoverySourceId.value) void loadDiscoveryCategories();
}

async function openDiscoveryArticle(result: SearchBookResult): Promise<void> {
  const sourceId = result.sourceId;
  if (!sourceId) return;
  const resultId = result.resultId;
  articleTitle.value = result.title;
  articleBusy.value = true;
  articleOpen.value = true;
  try {
    const response = await openRssArticle(sourceId, resultId);
    const rawHtml = await readTextResource(response.resource);
    articleHtml.value = makeDisplayHtml(rawHtml, settings.value.reader, response.resource.src);
  } catch (error) {
    articleOpen.value = false;
    notify(`打开文章失败：${errorText(error)}`, "error");
  } finally {
    articleBusy.value = false;
  }
}

async function loadDiscoveryCategoriesAfterNav(): Promise<void> {
  if (!discoverySourceId.value) discoverySourceId.value = enabledSources.value[0]?.id ?? "";
  if (discoverySourceId.value && !discoveryWasLoaded.value) await loadDiscoveryCategories();
}

function setAllSearchSources(enabled: boolean): void {
  selectedSourceIds.value = enabled ? enabledSources.value.map((source) => source.id) : [];
}

function setReaderTheme(theme: ReaderTheme): void {
  const colors: Record<string, { textColor: string; backgroundColor: string }> = {
    paper: { textColor: "#3f3b34", backgroundColor: "#f7f3e9" },
    sepia: { textColor: "#4e4131", backgroundColor: "#eee1c8" },
    dark: { textColor: "#d1d0cb", backgroundColor: "#17191c" },
    system: { textColor: "#252525", backgroundColor: "#ffffff" },
  };
  settings.value = {
    ...settings.value,
    reader: { ...settings.value.reader, theme, ...colors[theme] },
  };
  if (readerVisible.value && readingChapterRaw.value) {
    readingChapterHtml.value = makeDisplayHtml(readingChapterRaw.value, settings.value.reader, currentChapter.value?.src, readingBook.value?.id);
  }
  scheduleSettingsSave();
}

function openSearchResult(result: SearchBookResult): void {
  selectedResult.value = result;
}

async function addSearchResult(result: SearchBookResult): Promise<void> {
  if (addBusyResult.value) return;
  addBusyResult.value = result.resultId;
  globalError.value = "";
  try {
    const response = await addBook(result.resultId);
    const [book, nextShelf] = await Promise.all([
      readResource<BookResource>(response.book),
      readResource<ShelfResource>(response.shelf),
    ]);
    openedBook.value = book;
    shelf.value = { ...nextShelf, books: nextShelf.books ?? [] };
    selectedResult.value = null;
    notify(`《${book.title}》已加入书架。`);
  } catch (error) {
    globalError.value = errorText(error);
  } finally {
    addBusyResult.value = null;
  }
}

async function importLocalBook(): Promise<void> {
  if (localBookImportBusy.value) return;
  localBookImportBusy.value = true;
  try {
    const response = await importBookFromPicker();
    if (response.cancelled) return;
    if (!response.book || !response.shelf) throw new Error("导入完成后没有返回书籍资源。");
    const [book, nextShelf] = await Promise.all([
      readResource<BookResource>(response.book),
      readResource<ShelfResource>(response.shelf),
    ]);
    openedBook.value = book;
    shelf.value = { ...nextShelf, books: nextShelf.books ?? [] };
    notify(`《${book.title}》已加入书架。`);
  } catch (error) {
    notify(`导入本地书失败：${errorText(error)}`, "error");
  } finally {
    localBookImportBusy.value = false;
  }
}

async function createBackup(): Promise<void> {
  if (backupBusy.value) return;
  backupBusy.value = "create";
  try {
    const result: BackupResponse = await createBackupFromPicker();
    if (result.cancelled) return;
    if (!result.backup) throw new Error("备份没有完成。");
    lastBackupAt.value = Date.now();
    notify("备份文件已保存。请妥善保管其中的书源与登录状态。");
  } catch (error) {
    notify(`创建备份失败：${errorText(error)}`, "error");
  } finally {
    backupBusy.value = null;
  }
}

async function restoreBackup(): Promise<void> {
  if (backupBusy.value) return;
  if (!window.confirm("恢复备份会替换当前书架、阅读数据、书源和登录状态。要继续选择备份文件吗？")) return;
  backupBusy.value = "restore";
  try {
    const result: BackupResponse = await restoreBackupFromPicker();
    if (result.cancelled) return;
    if (!result.restored || !result.bootstrap) throw new Error("备份没有恢复完成。");
    const [nextShelf, nextSettings] = await Promise.all([
      readResource<ShelfResource>(result.bootstrap.shelf),
      readResource<AppSettingsResource>(result.bootstrap.settings),
    ]);
    shelf.value = { ...nextShelf, groups: nextShelf.groups ?? [], books: nextShelf.books ?? [] };
    settings.value = normalizeSettings(nextSettings);
    sources.value = result.bootstrap.sources ?? [];
    selectedSourceIds.value = enabledSources.value.map((source) => source.id);
    openedBook.value = null;
    searchResults.value = [];
    discoveryResults.value = [];
    await Promise.all([refreshReadingData(), refreshTasks()]);
    notify("备份已恢复，书架和阅读数据已重新载入。");
  } catch (error) {
    notify(`恢复备份失败：${errorText(error)}`, "error");
  } finally {
    backupBusy.value = null;
  }
}

async function openShelfBook(bookId: string): Promise<void> {
  bookPanelBusy.value = true;
  globalError.value = "";
  try {
    const descriptor = await getBook(bookId);
    openedBook.value = await readResource<BookResource>(descriptor);
  } catch (error) {
    globalError.value = errorText(error);
  } finally {
    bookPanelBusy.value = false;
  }
}

async function refreshOpenedBook(descriptor?: ResourceDescriptor): Promise<void> {
  const id = readingBook.value?.id ?? openedBook.value?.id;
  if (!id && !descriptor) return;
  const resource = descriptor ?? await getBook(id!);
  const book = await readResource<BookResource>(resource);
  if (readingBook.value?.id === book.id) readingBook.value = book;
  if (openedBook.value?.id === book.id) openedBook.value = book;
}

async function removeBookFromShelf(): Promise<void> {
  if (!pendingRemoval.value) return;
  const { id, title } = pendingRemoval.value;
  globalError.value = "";
  try {
    const response = await removeBook(id);
    const nextShelf = await readResource<ShelfResource>(response.shelf);
    shelf.value = { ...nextShelf, books: nextShelf.books ?? [] };
    pendingRemoval.value = null;
    if (openedBook.value?.id === id) openedBook.value = null;
    notify(`《${title}》已从书架移除。`);
  } catch (error) {
    globalError.value = errorText(error);
  }
}

async function startReading(book: BookResource, requestedIndex?: number, requestedOffset?: number): Promise<void> {
  const savedIndex = book.progress?.chapterIndex ?? 0;
  const firstIndex = Math.max(0, Math.min(book.chapterCount - 1, requestedIndex ?? savedIndex));
  readingBook.value = book;
  readingChapterIndex.value = firstIndex;
  readerPageIndex.value = requestedOffset != null
    ? Math.max(0, requestedOffset)
    : requestedIndex == null && book.progress?.chapterIndex === firstIndex
      ? Math.max(0, book.progress.offset ?? 0)
      : 0;
  readerVisible.value = true;
  readerControlsOpen.value = false;
  screen.value = "shelf";
  readerProgressDirty.value = false;
  progressSaveState.value = "idle";
  await loadReaderChapter(firstIndex);
  if (requestedOffset != null && readingChapterRaw.value) {
    readerPageIndex.value = Math.min(Math.max(0, requestedOffset), readerPageCount.value - 1);
    readerProgressDirty.value = true;
    progressRevision += 1;
    scrollReaderFrameToPage();
  }
  if (readingChapterRaw.value) beginReadingSession();
}

function beginReadingSession(): void {
  if (!readingBook.value || readingSessionId.value) return;
  readingSessionId.value = globalThis.crypto?.randomUUID?.() ?? `session-${Date.now()}-${Math.random().toString(16).slice(2)}`;
  readingSessionStartedAt.value = Date.now();
}

async function finishReadingSession(): Promise<void> {
  const bookId = readingBook.value?.id;
  const sessionId = readingSessionId.value;
  const startedAt = readingSessionStartedAt.value;
  readingSessionId.value = null;
  readingSessionStartedAt.value = null;
  if (!bookId || !sessionId || startedAt == null) return;
  const endedAtMs = Date.now();
  try {
    const descriptor = await recordReadingSession({
      bookId,
      sessionId,
      durationMs: Math.max(0, endedAtMs - startedAt),
      endedAtMs,
      utcOffsetMinutes: -new Date(endedAtMs).getTimezoneOffset(),
    });
    const updated = await readResource<ReadingHistoryResource>(descriptor);
    readingHistory.value = updated;
  } catch (error) {
    notify(`保存阅读记录失败：${errorText(error)}`, "error");
  }
}

async function openBookmark(bookmark: Bookmark): Promise<void> {
  try {
    const descriptor = await getBook(bookmark.bookId);
    const book = await readResource<BookResource>(descriptor);
    screen.value = "shelf";
    await startReading(book, bookmark.chapterIndex, bookmark.offset);
  } catch (error) {
    notify(`打开书签失败：${errorText(error)}`, "error");
  }
}

async function addBookmark(): Promise<void> {
  const book = readingBook.value;
  if (!book || bookmarkBusy.value) return;
  bookmarkBusy.value = true;
  try {
    const descriptor = await upsertBookmark({
      bookId: book.id,
      chapterIndex: readingChapterIndex.value,
      offset: readerPageIndex.value,
      note: bookmarkNote.value.trim(),
    });
    const resource = await readResource<BookmarksResource>(descriptor);
    bookmarks.value = resource.bookmarks ?? [];
    bookmarkEditorOpen.value = false;
    bookmarkNote.value = "";
    notify("书签已保存。");
  } catch (error) {
    notify(`保存书签失败：${errorText(error)}`, "error");
  } finally {
    bookmarkBusy.value = false;
  }
}

async function removeBookmark(bookmarkId: string): Promise<void> {
  try {
    const descriptor = await deleteBookmark(bookmarkId);
    const resource = await readResource<BookmarksResource>(descriptor);
    bookmarks.value = resource.bookmarks ?? [];
    notify("书签已删除。");
  } catch (error) {
    notify(`删除书签失败：${errorText(error)}`, "error");
  }
}

async function clearReadingHistoryData(): Promise<void> {
  try {
    const descriptor = await clearReadingHistory();
    readingHistory.value = await readResource<ReadingHistoryResource>(descriptor);
    notify("阅读记录已清空。");
  } catch (error) {
    notify(`清空阅读记录失败：${errorText(error)}`, "error");
  }
}

async function removeBookHistory(bookId: string): Promise<void> {
  try {
    const descriptor = await deleteReadingHistoryForBook(bookId);
    readingHistory.value = await readResource<ReadingHistoryResource>(descriptor);
    notify("这本书的阅读记录已删除。");
  } catch (error) {
    notify(`删除阅读记录失败：${errorText(error)}`, "error");
  }
}

async function ensureChapterResource(bookId: string, index: number): Promise<BookResource> {
  let book = readingBook.value;
  if (!book || book.id !== bookId) throw new Error("当前书籍状态已失效，请重新打开。");
  let chapter = book.chapters.find((entry) => entry.index === index);
  if (chapter?.src) return book;

  if (preparePromise) await preparePromise;
  book = readingBook.value;
  chapter = book?.chapters.find((entry) => entry.index === index);
  if (chapter?.src && book) return book;

  prefetchBusy.value = true;
  preparePromise = (async () => {
    const count = Math.max(1, Math.min(20, settings.value.reader.preloadCount || 1));
    const response = await prepareChapters(bookId, index, count);
    const updatedBook = await readResource<BookResource>(response.book);
    readingBook.value = updatedBook;
    if (openedBook.value?.id === bookId) openedBook.value = updatedBook;
  })();
  try {
    await preparePromise;
  } finally {
    preparePromise = null;
    prefetchBusy.value = false;
  }
  book = readingBook.value;
  chapter = book?.chapters.find((entry) => entry.index === index);
  if (!book || !chapter?.src) throw new Error("章节准备失败，请返回目录后重试。");
  return book;
}

async function requestUpcomingChapters(book: BookResource, fromIndex: number): Promise<void> {
  const total = Math.max(1, book.chapterCount || book.chapters.length);
  if (fromIndex >= total || prefetchBusy.value) return;
  const wanted = Math.min(settings.value.reader.preloadCount, total - fromIndex);
  const prepared = book.chapters.filter((chapter) => chapter.index >= fromIndex && chapter.index < fromIndex + wanted && chapter.src).length;
  if (prepared >= wanted) return;
  prefetchBusy.value = true;
  try {
    const response = await prepareChapters(book.id, fromIndex, wanted);
    const updated = await readResource<BookResource>(response.book);
    if (readingBook.value?.id === book.id) readingBook.value = updated;
    if (openedBook.value?.id === book.id) openedBook.value = updated;
  } catch (error) {
    notify(`后续章节缓存失败：${errorText(error)}`, "error");
  } finally {
    prefetchBusy.value = false;
  }
}

async function readChapterWithRecovery(book: BookResource, index: number): Promise<{ book: BookResource; chapter: BookResource["chapters"][number]; html: string }> {
  let chapter = book.chapters.find((entry) => entry.index === index);
  if (!chapter?.src) throw new Error(`第 ${index + 1} 章没有可读取的资源地址。`);
  try {
    return { book, chapter, html: await readTextResource(chapter.src) };
  } catch (error) {
    if (!(error instanceof ResourceHttpError) || error.status !== 404) throw error;
    notify("章节暂时无法打开，正在重新获取。", "error");
    const response = await prepareChapters(book.id, index, Math.max(1, settings.value.reader.preloadCount));
    const refreshedBook = await readResource<BookResource>(response.book);
    readingBook.value = refreshedBook;
    if (openedBook.value?.id === refreshedBook.id) openedBook.value = refreshedBook;
    chapter = refreshedBook.chapters.find((entry) => entry.index === index);
    if (!chapter?.src) throw new Error("章节暂时不可用，请稍后重试。");
    return { book: refreshedBook, chapter, html: await readTextResource(chapter.src) };
  }
}

async function loadReaderChapter(index: number): Promise<void> {
  if (!readingBook.value) return;
  readerBusy.value = true;
  try {
    const preparedBook = await ensureChapterResource(readingBook.value.id, index);
    const loaded = await readChapterWithRecovery(preparedBook, index);
    const book = loaded.book;
    const chapter = loaded.chapter;
    const rawHtml = loaded.html;
    readingChapterIndex.value = index;
    readingChapterRaw.value = rawHtml;
    readerPageIndex.value = book.progress?.chapterIndex === index ? Math.max(0, book.progress.offset ?? 0) : 0;
    readingChapterHtml.value = makeDisplayHtml(rawHtml, settings.value.reader, chapter.src, book.id);
    readerPageCount.value = 1;
    readerProgressDirty.value = true;
    progressRevision += 1;
    progressSaveState.value = "idle";
    await nextTick();
    measureReaderPages();
    if (index + 1 < Math.max(book.chapterCount, book.chapters.length)) void requestUpcomingChapters(book, index + 1);
  } catch (error) {
    globalError.value = `无法打开章节：${errorText(error)}`;
    notify("章节资源读取失败。", "error");
  } finally {
    readerBusy.value = false;
  }
}

function makeDisplayHtml(raw: string, reader: ReaderSettings, chapterSrc?: string, bookId?: string): string {
  const document = new DOMParser().parseFromString(raw, "text/html");
  document.querySelectorAll("script, iframe, object, embed, form, base, meta[http-equiv='refresh']").forEach((node) => node.remove());
  document.querySelectorAll("meta[http-equiv], link").forEach((node) => {
    if (node instanceof HTMLMetaElement) {
      const directive = node.httpEquiv.toLocaleLowerCase();
      if (directive === "content-security-policy" || directive === "refresh") node.remove();
    } else if (node instanceof HTMLLinkElement && !node.relList.contains("stylesheet")) {
      node.remove();
    }
  });
  document.querySelectorAll("*").forEach((node) => {
    for (const attribute of [...node.attributes]) {
      if (/^on/i.test(attribute.name) || ((attribute.name === "href" || attribute.name === "src") && /^\s*javascript:/i.test(attribute.value))) {
        node.removeAttribute(attribute.name);
      }
    }
  });
  if (chapterSrc) rebaseRelativeResources(document, chapterSrc);
  applyReaderReplacements(document.body, replacementRules.value, bookId);

  document.head.prepend(makeReaderCsp(document, chapterSrc));
  const style = document.createElement("style");
  style.dataset.legadoReaderOverride = "true";
  style.textContent = `
    html,body{width:100%;height:100%;min-height:100%;margin:0!important;overflow:hidden!important;}
    body{--reader-gutter:min(8vw,96px);box-sizing:border-box!important;height:100vh!important;padding:28px var(--reader-gutter)!important;
      background:${safeCssColor(reader.backgroundColor, "#f7f3e9")}!important;color:${safeCssColor(reader.textColor, "#3f3b34")}!important;
      column-width:calc(100vw - min(16vw,192px))!important;column-gap:min(16vw,192px)!important;
      column-fill:auto!important;overflow-x:auto!important;overflow-y:hidden!important;
      font-family:${cssFont(reader.fontFamily)}!important;font-size:${clamp(reader.fontSizePx,12,36)}px!important;
      line-height:${clamp(reader.lineHeight,1.1,3)}!important;text-align:${safeTextAlign(reader.textAlign)}!important;scrollbar-width:none!important;}
    body::-webkit-scrollbar{display:none!important;}
    body *{max-width:100%!important;line-height:${clamp(reader.lineHeight,1.1,3)}!important;
      font-family:${cssFont(reader.fontFamily)}!important;font-size:inherit!important;}
    p,li,blockquote{break-inside:auto!important;white-space:normal!important;}
    img,video,svg{max-width:100%!important;height:auto!important;object-fit:contain!important;}
    a{color:inherit!important;}
  `;
  (document.head ?? document.documentElement).append(style);
  return `<!doctype html>${document.documentElement.outerHTML}`;
}

function makeReaderCsp(document: Document, chapterSrc?: string): HTMLMetaElement {
  let resourceOrigin = "";
  if (chapterSrc) {
    try {
      const parsed = new URL(chapterSrc);
      if (parsed.protocol === "http:" || parsed.protocol === "https:") resourceOrigin = parsed.origin;
    } catch {
      resourceOrigin = "";
    }
  }
  const allowedAssets = [resourceOrigin, "http:", "https:", "data:", "blob:"].filter(Boolean).join(" ");
  const policy = document.createElement("meta");
  policy.httpEquiv = "Content-Security-Policy";
  policy.content = [
    "default-src 'none'",
    `img-src ${allowedAssets}`,
    `media-src ${allowedAssets}`,
    `style-src 'unsafe-inline' ${resourceOrigin} http: https:`,
    `font-src ${resourceOrigin} http: https: data:`,
    "script-src 'none'",
    "connect-src 'none'",
    "frame-src 'none'",
    "object-src 'none'",
    "base-uri 'none'",
    "form-action 'none'",
  ].join("; ");
  return policy;
}

function rebaseRelativeResources(document: Document, chapterSrc: string): void {
  const resolve = (value: string): string => {
    const trimmed = value.trim();
    if (!trimmed || /^(?:[a-z][a-z\d+.-]*:|#)/i.test(trimmed)) return value;
    try {
      return new URL(trimmed, chapterSrc).href;
    } catch {
      return value;
    }
  };
  document.querySelectorAll<HTMLElement>("[src], [href], [poster]").forEach((node) => {
    for (const attribute of ["src", "href", "poster"]) {
      const value = node.getAttribute(attribute);
      if (value) node.setAttribute(attribute, resolve(value));
    }
    const srcset = node.getAttribute("srcset");
    if (srcset) {
      const candidates = srcset.match(/data:[^\s,]+,[^\s]+(?:\s+\d+(?:\.\d+)?[wx])?|[^,\s]+(?:\s+\d+(?:\.\d+)?[wx])?/gi);
      if (candidates?.length) {
        node.setAttribute("srcset", candidates.map((candidate) => {
          const split = candidate.trim().match(/^(\S+)(\s+\d+(?:\.\d+)?[wx])?$/i);
          return split ? `${resolve(split[1])}${split[2] ?? ""}` : candidate;
        }).join(", "));
      }
    }
    const inlineStyle = node.getAttribute("style");
    if (inlineStyle) node.setAttribute("style", rebaseCssUrls(inlineStyle, resolve));
  });
  document.querySelectorAll("style").forEach((style) => {
    style.textContent = rebaseCssUrls(style.textContent ?? "", resolve);
  });
}

function rebaseCssUrls(css: string, resolve: (value: string) => string): string {
  return css.replace(/url\(\s*(["']?)(.*?)\1\s*\)/gi, (_match, quote: string, value: string) => {
    const resolved = resolve(value);
    return `url(${quote}${resolved}${quote})`;
  });
}

function safeCssColor(value: string, fallback: string): string {
  return /^#[\da-f]{3,8}$/i.test(value) || /^(rgb|rgba|hsl|hsla)\([\d\s.,%+-]+\)$/i.test(value) ? value : fallback;
}

function safeTextAlign(value: string): "left" | "right" | "center" | "justify" {
  return ["left", "right", "center", "justify"].includes(value)
    ? value as "left" | "right" | "center" | "justify"
    : "justify";
}

function cssFont(font: string): string {
  const fonts: Record<string, string> = {
    serif: "Georgia, 'Noto Serif', 'Songti SC', serif",
    sans: "system-ui, -apple-system, 'Segoe UI', sans-serif",
    system: "system-ui, -apple-system, 'Segoe UI', sans-serif",
    mono: "ui-monospace, 'SFMono-Regular', monospace",
  };
  return fonts[font] ?? "system-ui, sans-serif";
}

function clamp(value: number, minimum: number, maximum: number): number {
  const number = Number(value);
  return Number.isFinite(number) ? Math.min(maximum, Math.max(minimum, number)) : minimum;
}

function applyReaderReplacements(root: HTMLElement, replacements: DisplayReplacementRule[], bookId?: string): void {
  const active = replacements.filter((rule) => rule.enabled && rule.pattern && (rule.scope === "all" || rule.scope === `book:${bookId}`));
  if (!active.length) return;
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode(node) {
      const parent = node.parentElement;
      return parent && !parent.closest("script,style,noscript,textarea") ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_REJECT;
    },
  });
  const textNodes: Text[] = [];
  while (walker.nextNode()) textNodes.push(walker.currentNode as Text);
  for (const node of textNodes) {
    let content = node.data;
    for (const rule of active) {
      if (rule.isRegex) {
        try {
          content = content.replace(new RegExp(rule.pattern, "g"), rule.replacement);
        } catch {
          // Invalid user patterns stay visible in settings and are ignored in the reader.
        }
      } else {
        content = content.split(rule.pattern).join(rule.replacement);
      }
    }
    node.data = content;
  }
}

function measureReaderPages(preserveRatio = false): void {
  const doc = readerFrame.value?.contentDocument;
  if (!doc) return;
  const body = doc.body;
  const width = Math.max(1, doc.documentElement.clientWidth || readerFrame.value?.clientWidth || 1);
  const previousCount = readerPageCount.value;
  const previousIndex = readerPageIndex.value;
  const totalWidth = Math.max(width, body.scrollWidth, doc.documentElement.scrollWidth);
  readerPageCount.value = Math.max(1, Math.ceil((totalWidth - 1) / width));
  readerPageIndex.value = preserveRatio && previousCount > 1
    ? Math.round((previousIndex / (previousCount - 1)) * (readerPageCount.value - 1))
    : Math.min(readerPageIndex.value, readerPageCount.value - 1);
  scrollReaderFrameToPage();
}

function scrollReaderFrameToPage(): void {
  const doc = readerFrame.value?.contentDocument;
  if (!doc) return;
  const width = Math.max(1, doc.documentElement.clientWidth || readerFrame.value?.clientWidth || 1);
  doc.body.scrollTo({ left: readerPageIndex.value * width, behavior: "instant" as ScrollBehavior });
  doc.documentElement.scrollLeft = readerPageIndex.value * width;
}

function turnPage(direction: -1 | 1): void {
  if (!readerVisible.value || readerBusy.value) return;
  const next = readerPageIndex.value + direction;
  if (next >= 0 && next < readerPageCount.value) {
    readerPageIndex.value = next;
    readerProgressDirty.value = true;
    progressRevision += 1;
    progressSaveState.value = "idle";
    scrollReaderFrameToPage();
    return;
  }
  if (direction > 0 && readerPageIndex.value === readerPageCount.value - 1) void changeChapter(1);
  else if (direction < 0 && readerPageIndex.value === 0) void changeChapter(-1);
}

async function changeChapter(direction: -1 | 1): Promise<void> {
  const nextIndex = readingChapterIndex.value + direction;
  const count = readingBook.value?.chapterCount ?? readingBook.value?.chapters.length ?? 0;
  if (nextIndex < 0) {
    notify("已经是第一章。");
    return;
  }
  if (nextIndex >= count) {
    notify("已经到达书源提供的最后一章。", "error");
    return;
  }
  await loadReaderChapter(nextIndex);
}

async function saveCurrentProgress(): Promise<void> {
  const book = readingBook.value;
  if (!book || !readerProgressDirty.value) return;
  if (progressSavePromise) {
    await progressSavePromise;
    if (readerProgressDirty.value && readerVisible.value) return saveCurrentProgress();
    return;
  }
  const saveRevision = progressRevision;
  let saveSucceeded = false;
  const progress: ReadingProgress = {
    chapterId: currentChapter.value?.id ?? null,
    chapterIndex: readingChapterIndex.value,
    offset: readerPageIndex.value,
    updatedAtMs: Date.now(),
  };
  progressSaveState.value = "saving";
  progressSavePromise = (async () => {
    try {
      await saveProgress(book.id, progress);
      saveSucceeded = true;
      if (progressRevision === saveRevision) readerProgressDirty.value = false;
      progressSaveState.value = "saved";
      if (readingBook.value?.id === book.id) {
        readingBook.value.progress = { ...progress };
      }
    } catch (error) {
      progressSaveState.value = "error";
      notify(`阅读进度保存失败：${errorText(error)}`, "error");
    } finally {
      progressSavePromise = null;
    }
  })();
  await progressSavePromise;
  if (saveSucceeded && readerProgressDirty.value && readerVisible.value) return saveCurrentProgress();
}

async function closeReader(): Promise<void> {
  await saveCurrentProgress();
  await finishReadingSession();
  readerVisible.value = false;
  readingChapterHtml.value = "";
  readerFrame.value = null;
  if (readingBook.value && openedBook.value?.id === readingBook.value.id) {
    try {
      await refreshOpenedBook();
    } catch (error) {
      notify(`刷新阅读位置失败：${errorText(error)}`, "error");
    }
  }
}

function updateReaderSetting<K extends keyof ReaderSettings>(key: K, value: ReaderSettings[K]): void {
  settings.value = { ...settings.value, reader: { ...settings.value.reader, [key]: value } };
  if (readerVisible.value && readingChapterHtml.value) {
    if (readingChapterRaw.value) {
      readingChapterHtml.value = makeDisplayHtml(readingChapterRaw.value, settings.value.reader, currentChapter.value?.src, readingBook.value?.id);
    }
  }
  scheduleSettingsSave();
}

function scheduleSettingsSave(): void {
  if (pendingSettingsSave.value) clearTimeout(pendingSettingsSave.value);
  pendingSettingsSave.value = setTimeout(() => void persistSettings(), 450);
}

async function persistSettings(): Promise<void> {
  if (pendingSettingsSave.value) clearTimeout(pendingSettingsSave.value);
  pendingSettingsSave.value = null;
  saveSettingsBusy.value = true;
  try {
    const normalized = normalizeSettings(settings.value);
    const descriptor = await saveSettings(normalized);
    settings.value = normalizeSettings(await readResource<AppSettingsResource>(descriptor));
    notify("阅读设置已保存。");
  } catch (error) {
    notify(`阅读设置保存失败：${errorText(error)}`, "error");
  } finally {
    saveSettingsBusy.value = false;
  }
}

function rebuildReaderDisplay(): void {
  if (!readerVisible.value || !readingChapterRaw.value) return;
  readingChapterHtml.value = makeDisplayHtml(readingChapterRaw.value, settings.value.reader, currentChapter.value?.src, readingBook.value?.id);
}

async function refreshReplacementRules(descriptor?: ResourceDescriptor): Promise<void> {
  const resource = descriptor ?? await listReplacementRules();
  const document = await readResource<{ schemaVersion: number; rules: DisplayReplacementRule[] }>(resource);
  replacementRules.value = Array.isArray(document.rules) ? document.rules : [];
  rebuildReaderDisplay();
}

async function addReplacement(): Promise<void> {
  try {
    const descriptor = await upsertReplacementRule({
      name: `文字替换 ${replacementRules.value.length + 1}`,
      pattern: "",
      replacement: "",
      enabled: true,
      isRegex: false,
      scope: "all",
    });
    await refreshReplacementRules(descriptor);
    notify("已添加显示替换。");
  } catch (error) {
    notify(`添加显示替换失败：${errorText(error)}`, "error");
  }
}

function updateReplacementRule(ruleId: string, patch: Partial<DisplayReplacementRule>): void {
  replacementRules.value = replacementRules.value.map((rule) => rule.id === ruleId ? { ...rule, ...patch } : rule);
  rebuildReaderDisplay();
  if (replacementSaveTimer) clearTimeout(replacementSaveTimer);
  replacementSaveTimer = setTimeout(() => {
    replacementSaveTimer = null;
    void persistReplacementRule(ruleId);
  }, 500);
}

async function persistReplacementRule(ruleId: string): Promise<void> {
  const previous = replacementWrites.get(ruleId) ?? Promise.resolve();
  const next = previous.catch(() => {}).then(async () => {
    const rule = replacementRules.value.find((entry) => entry.id === ruleId);
    if (!rule) return;
    const descriptor = await upsertReplacementRule({ ...rule });
    if (replacementRules.value.some((entry) => entry.id === ruleId)) {
      const document = await readResource<{ schemaVersion: number; rules: DisplayReplacementRule[] }>(descriptor);
      replacementRules.value = replacementRules.value.map((entry) => entry.id === ruleId
        ? (document.rules.find((saved) => saved.id === ruleId) ?? entry)
        : entry);
    }
    rebuildReaderDisplay();
  });
  replacementWrites.set(ruleId, next);
  try {
    await next;
  } catch (error) {
    notify(`保存显示替换失败：${errorText(error)}`, "error");
  } finally {
    if (replacementWrites.get(ruleId) === next) replacementWrites.delete(ruleId);
  }
}

async function removeReplacement(ruleId: string): Promise<void> {
  if (replacementSaveTimer) {
    clearTimeout(replacementSaveTimer);
    replacementSaveTimer = null;
  }
  await replacementWrites.get(ruleId)?.catch(() => {});
  try {
    const descriptor = await deleteReplacementRule(ruleId);
    replacementRules.value = replacementRules.value.filter((rule) => rule.id !== ruleId);
    await refreshReplacementRules(descriptor);
    notify("显示替换已移除。");
  } catch (error) {
    notify(`删除显示替换失败：${errorText(error)}`, "error");
  }
}

async function importSourceFileFromPicker(): Promise<void> {
  if (sourceImportBusy.value) return;
  sourceImportBusy.value = true;
  try {
    const response = await importSourcesFromPicker();
    if (response.cancelled) return;
    sources.value = response.sources;
    selectedSourceIds.value = enabledSources.value.map((source) => source.id);
    sourceImportOpen.value = false;
    notify("书源已导入。");
  } catch (error) {
    notify(`书源导入失败：${errorText(error)}`, "error");
  } finally {
    sourceImportBusy.value = false;
  }
}

async function deleteSelectedSources(): Promise<void> {
  if (!pendingSourceRemoval.value.length) return;
  try {
    await removeSources(pendingSourceRemoval.value);
    await refreshSources();
    selectedSourceRows.value = [];
    pendingSourceRemoval.value = [];
    notify("所选书源已移除。");
  } catch (error) {
    notify(`删除书源失败：${errorText(error)}`, "error");
  }
}

function toggleSourceRow(id: string): void {
  selectedSourceRows.value = selectedSourceRows.value.includes(id)
    ? selectedSourceRows.value.filter((item) => item !== id)
    : [...selectedSourceRows.value, id];
}

function toggleSearchSource(id: string): void {
  selectedSourceIds.value = selectedSourceIds.value.includes(id)
    ? selectedSourceIds.value.filter((item) => item !== id)
    : [...selectedSourceIds.value, id];
}

function sourceName(id: string): string {
  return sources.value.find((source) => source.id === id)?.name ?? id;
}

function onGlobalKeydown(event: KeyboardEvent): void {
  if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
    event.preventDefault();
    if (!readerVisible.value) chooseScreen("search");
    return;
  }
  if (!readerVisible.value || event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement) return;
  if (event.key === "ArrowRight" || event.key === "PageDown" || event.key === " ") {
    event.preventDefault();
    turnPage(1);
  } else if (event.key === "ArrowLeft" || event.key === "PageUp") {
    event.preventDefault();
    turnPage(-1);
  } else if (event.key === "Escape") {
    event.preventDefault();
    void closeReader();
  }
}

function onVisibilityChange(): void {
  if (document.visibilityState === "hidden") {
    void saveCurrentProgress();
    void finishReadingSession();
  } else if (readerVisible.value) {
    beginReadingSession();
  }
}

function onPageHide(): void {
  void saveCurrentProgress();
  void finishReadingSession();
}

async function setupEvents(): Promise<void> {
  try {
    unlisteners.push(await listen<{ sourceId?: string; sourceName?: string; completedSources: number; totalSources: number; resultCount: number }>("search-progress", (event) => {
      const payload = event.payload;
      searchProgress.value = `${payload.sourceName ?? sourceName(payload.sourceId ?? "")} · ${payload.completedSources}/${payload.totalSources} 个书源 · ${payload.resultCount} 本`;
    }));
    unlisteners.push(await listen<{ bookId: string; book: ResourceDescriptor }>("chapters-prepared", async (event) => {
      const { bookId, book } = event.payload;
      if (readingBook.value?.id === bookId || openedBook.value?.id === bookId) {
        try {
          await refreshOpenedBook(book);
        } catch (error) {
          notify(`读取更新后的目录失败：${errorText(error)}`, "error");
        }
      }
    }));
    unlisteners.push(await listen<SourceMetadata[]>("sources-updated", (event) => {
      sources.value = event.payload;
      selectedSourceIds.value = selectedSourceIds.value.filter((id) => enabledSources.value.some((source) => source.id === id));
      if (!enabledSources.value.some((source) => source.id === discoverySourceId.value)) {
        discoverySourceId.value = enabledSources.value[0]?.id ?? "";
        discoveryCategories.value = [];
        discoveryResults.value = [];
      }
    }));
    unlisteners.push(await listen<ResourceDescriptor>("shelf-updated", async (event) => {
      try {
        const nextShelf = await readResource<ShelfResource>(event.payload);
        shelf.value = { ...nextShelf, groups: nextShelf.groups ?? [], books: nextShelf.books ?? [] };
      } catch (error) {
        notify(`书架更新失败：${errorText(error)}`, "error");
      }
    }));
    unlisteners.push(await listen<ResourceDescriptor>("settings-updated", async (event) => {
      try {
        settings.value = normalizeSettings(await readResource<AppSettingsResource>(event.payload));
      } catch (error) {
        notify(`阅读设置更新失败：${errorText(error)}`, "error");
      }
    }));
    unlisteners.push(await listen<{ kind: string; resource: ResourceDescriptor }>("resource-updated", async (event) => {
      try {
        if (event.payload.kind === "bookmarks") {
          const document = await readResource<BookmarksResource>(event.payload.resource);
          bookmarks.value = document.bookmarks ?? [];
        } else if (event.payload.kind === "readingHistory") {
          readingHistory.value = await readResource<ReadingHistoryResource>(event.payload.resource);
        } else if (event.payload.kind === "replacementRules") {
          await refreshReplacementRules(event.payload.resource);
        }
      } catch (error) {
        notify(`阅读数据更新失败：${errorText(error)}`, "error");
      }
    }));
    unlisteners.push(await listen<{ task: Partial<AppTask>; resource: ResourceDescriptor }>("task-updated", async (event) => {
      try {
        await refreshTasks(event.payload.resource);
        await syncActiveSearchTask(event.payload.task);
      } catch (error) {
        notify(`任务状态更新失败：${errorText(error)}`, "error");
      }
    }));
    unlisteners.push(await listen<AppBootstrap>("app-state-updated", async (event) => {
      try {
        const restored = event.payload;
        const [nextShelf, nextSettings] = await Promise.all([
          readResource<ShelfResource>(restored.shelf),
          readResource<AppSettingsResource>(restored.settings),
        ]);
        shelf.value = { ...nextShelf, groups: nextShelf.groups ?? [], books: nextShelf.books ?? [] };
        settings.value = normalizeSettings(nextSettings);
        sources.value = restored.sources ?? [];
        selectedSourceIds.value = enabledSources.value.map((source) => source.id);
        await Promise.all([refreshReadingData(), refreshTasks()]);
      } catch (error) {
        notify(`恢复后的数据重新载入失败：${errorText(error)}`, "error");
      }
    }));
    unlisteners.push(await listen<{ bookId: string; book: ResourceDescriptor }>("progress-saved", async (event) => {
      try {
        const updated = await readResource<BookResource>(event.payload.book);
        if (readingBook.value?.id === event.payload.bookId) readingBook.value = updated;
        if (openedBook.value?.id === event.payload.bookId) openedBook.value = updated;
      } catch (error) {
        notify(`阅读位置更新失败：${errorText(error)}`, "error");
      }
    }));
    unlisteners.push(await listen<{ resource: ResourceDescriptor; bookCount: number; errors: Array<{ sourceId?: string; message: string }> }>("search-complete", async (event) => {
      if (!searchWasRun.value || !searchKeyword.value.trim()) return;
      try {
        await loadSearchResource(event.payload.resource);
        searchResponseErrors.value = event.payload.errors ?? [];
        searchProgress.value = `搜索完成 · ${event.payload.bookCount} 本书`;
      } catch (error) {
        notify(`读取搜索结果失败：${errorText(error)}`, "error");
      }
    }));
  } catch (error) {
    // App events are optional for a browser preview; every command result remains authoritative.
    console.debug("Native event subscription unavailable:", errorText(error));
  }
}

onMounted(() => {
  void setupEvents();
  void bootstrap();
  window.addEventListener("keydown", onGlobalKeydown);
  window.addEventListener("pagehide", onPageHide);
  document.addEventListener("visibilitychange", onVisibilityChange);
  window.addEventListener("resize", handleReaderResize);
});

onBeforeUnmount(() => {
  window.removeEventListener("keydown", onGlobalKeydown);
  window.removeEventListener("pagehide", onPageHide);
  document.removeEventListener("visibilitychange", onVisibilityChange);
  window.removeEventListener("resize", handleReaderResize);
  for (const unlisten of unlisteners) unlisten();
  if (toastTimer) clearTimeout(toastTimer);
  if (pendingSettingsSave.value) clearTimeout(pendingSettingsSave.value);
  if (replacementSaveTimer) {
    clearTimeout(replacementSaveTimer);
    for (const rule of replacementRules.value) void persistReplacementRule(rule.id);
  }
  void saveCurrentProgress();
  void finishReadingSession();
});

watch(readerSettings, () => {
  if (readerVisible.value && readingChapterHtml.value) measureReaderPages();
});
</script>
<template>
  <div class="app-shell" :class="{ 'reader-open': readerVisible }">
    <aside v-if="!readerVisible" class="side-rail">
      <div class="brand-mark" aria-label="Legado"><span>阅</span></div>
      <nav class="rail-links" aria-label="主导航">
        <button data-testid="nav-home" class="rail-link" :class="{ active: screen === 'home' }" @click="chooseScreen('home')"><span class="glyph">⌂</span><span>主页</span></button>
        <button data-testid="nav-shelf" class="rail-link" :class="{ active: screen === 'shelf' }" @click="chooseScreen('shelf')"><span class="glyph">▤</span><span>书架</span></button>
        <button data-testid="nav-search" class="rail-link" :class="{ active: screen === 'search' }" @click="chooseScreen('search')"><span class="glyph">⌕</span><span>发现</span></button>
        <button data-testid="nav-settings" class="rail-link" :class="{ active: screen === 'settings' || screen === 'sources' }" @click="chooseScreen('settings')"><span class="glyph">☻</span><span>我的</span></button>
      </nav>
      <div class="rail-bottom"><span class="connection-dot" :class="{ online: bootstrapped }"></span><small>{{ bootstrapped ? '已就绪' : '连接中' }}</small></div>
    </aside>

    <main v-if="!readerVisible" class="main-shell">
      <header class="app-header">
        <div class="header-context">
          <span class="overline">{{ screen === 'home' ? '主页' : screen === 'shelf' ? '我的书架' : screen === 'search' ? '发现' : screen === 'sources' ? '书源管理' : '我的' }}</span>
          <h1>{{ screen === 'home' ? '主页' : screen === 'shelf' ? '书架' : screen === 'search' ? '发现' : screen === 'sources' ? '书源管理' : '我的' }}</h1>
        </div>
        <div class="header-actions">
          <label v-if="screen === 'shelf'" class="header-search">
            <span>⌕</span><input v-model="shelfQuery" aria-label="筛选书架" placeholder="搜索书架" /><kbd>⌘ K</kbd>
          </label>
          <button v-if="screen === 'sources'" class="button primary small" @click="sourceImportOpen = true"><span>＋</span> 导入书源</button>
          <button v-if="screen === 'shelf'" class="round-icon-button" aria-label="搜索书籍" title="发现好书" @click="chooseScreen('search')">⌕</button>
          <span class="profile-chip"><span class="profile-avatar">L</span><span>我的图书馆</span></span>
        </div>
      </header>

      <div class="content-scroll">
        <div v-if="globalError" class="notice error-notice" role="alert">
          <span>!</span><div><strong>操作未完成</strong><p>{{ globalError }}</p></div>
          <button aria-label="关闭错误" @click="globalError = ''">×</button>
        </div>

        <section v-if="!bootstrapped" class="connection-state">
          <div class="connection-card">
            <div class="connection-symbol">{{ isLoading ? '…' : '!' }}</div>
            <h2>{{ isLoading ? '正在打开你的阅读空间' : '暂时无法读取书架' }}</h2>
            <p>{{ isLoading ? '正在读取书架、设置和书源。' : globalError || '请检查应用状态后重试。' }}</p>
            <button class="button primary" :disabled="isLoading" @click="bootstrap">{{ isLoading ? '加载中…' : '重新加载' }}</button>
          </div>
        </section>

        <template v-else>
          <section v-if="screen === 'home'" class="home-view">
            <div class="home-summary-row">
              <div><h2>继续上次阅读</h2><p>{{ recentBooks.length ? '最近读过的书和阅读位置在这里。' : '书架、书签和阅读记录会集中显示在主页。' }}</p></div>
              <div class="home-quick-actions"><button class="button secondary" @click="chooseScreen('shelf')">打开书架</button><button class="button primary" @click="chooseScreen('search')">发现书籍</button></div>
            </div>
            <div v-if="recentBooks.length" class="home-recent-list">
              <article v-for="book in recentBooks" :key="book.id" class="home-recent-book">
                <button class="home-book-cover" @click="continueReading(book.id)"><img v-if="book.coverSrc" :src="book.coverSrc" alt="" /><span v-else>{{ book.title.slice(0, 1) }}</span></button>
                <div class="home-book-copy"><strong>{{ book.title }}</strong><span>{{ book.author || '作者未知' }}</span><small>读到第 {{ (book.progress?.chapterIndex ?? 0) + 1 }} 章 · {{ book.latestChapter || '目录已更新' }}</small><div class="book-progress-track"><span :style="{ width: `${book.chapterCount ? Math.min(100, ((book.progress?.chapterIndex ?? 0) + 1) / book.chapterCount * 100) : 0}%` }"></span></div></div>
                <button class="button primary home-continue" @click="continueReading(book.id)">继续阅读</button>
              </article>
            </div>
            <div v-else class="home-empty"><p>还没有阅读进度</p><span>从书架打开一本书，阅读位置会自动记在这里。</span><button class="button secondary" @click="chooseScreen('shelf')">前往书架</button></div>
            <div class="home-lower-grid"><section class="home-summary-card"><strong>最近 90 天</strong><span>累计阅读 {{ formatReadingDuration(readingHistory.totalDurationMs) }}</span><small>{{ readingHistory.totalSessions }} 次阅读 · {{ readingHistory.books.length }} 本书</small><button class="text-button" @click="chooseScreen('settings')">查看阅读记录</button></section><section class="home-summary-card"><strong>书架藏书</strong><span>{{ shelf.books.length }} 本</span><small>{{ enabledSources.length }} 个已启用书源</small><button class="text-button" @click="chooseScreen('sources')">管理书源</button></section></div>
          </section>

          <section v-if="screen === 'shelf'" class="shelf-view">
            <div class="shelf-page-heading">
              <div><h2>我的书架</h2><p>{{ shelf.books.length }} 本书 · {{ shelfBooks.length }} 本显示</p></div>
              <div class="shelf-page-actions"><button data-testid="local-book-import" class="button secondary" :disabled="localBookImportBusy" @click="importLocalBook">{{ localBookImportBusy ? '正在导入…' : '导入 TXT / EPUB' }}</button><button class="button primary" @click="chooseScreen('search')">添加书籍</button></div>
            </div>

            <ShelfOrganizer :groups="shelfGroups" :active-group="shelfActiveGroup" :sort-key="shelfSortKey" :sort-order="shelfSortOrder" @filter-group="shelfActiveGroup = $event" @sort-change="changeShelfSort($event)" @order-change="changeShelfSort(shelfSortKey, $event)" @create-group="createGroup" @rename-group="renameGroup" @delete-group="removeShelfGroup" />
            <div v-if="shelfBooks.length" class="book-grid">
              <article v-for="book in shelfBooks" :key="book.id" :data-testid="`shelf-book-${book.id}`" class="shelf-card">
                <button class="cover-button" :aria-label="`打开《${book.title}》`" @click="openShelfBook(book.id)">
                  <img v-if="book.coverSrc" :src="book.coverSrc" alt="" class="book-cover" loading="lazy" />
                  <span v-else class="cover-fallback" :class="`cover-tone-${Math.abs(book.title.length) % 5}`"><span>LEGADO</span><strong>{{ book.title }}</strong><small>{{ book.author || '佚名' }}</small></span>
                  <span class="cover-shade"></span><span class="continue-chip">{{ book.progress?.chapterIndex != null ? '继续阅读' : '开始阅读' }} <b>→</b></span>
                </button>
                <div class="shelf-card-info">
                  <button class="book-title-button" @click="openShelfBook(book.id)">{{ book.title }}</button>
                  <span class="book-author">{{ book.author || '作者未知' }}</span>
                  <div class="book-card-meta"><span>共 {{ book.chapterCount || '—' }} 章</span><span v-if="book.latestChapter">最新 · {{ book.latestChapter }}</span></div>
                  <div class="book-progress-track"><span :style="{ width: `${book.progress?.chapterIndex != null && book.chapterCount ? Math.min(100, ((book.progress.chapterIndex + 1) / book.chapterCount) * 100) : 0}%` }"></span></div>
                </div>
                <button class="card-menu" :aria-label="`从书架移除《${book.title}》`" title="从书架移除" @click="pendingRemoval = { id: book.id, title: book.title }">···</button>
              </article>
            </div>
            <div v-else-if="shelf.books.length" class="empty-card compact-empty"><div class="empty-icon">⌕</div><h3>没有找到这本书</h3><p>换个书名或作者试试。</p><button class="button secondary" @click="shelfQuery = ''">清除筛选</button></div>
            <div v-else class="shelf-empty-compact"><p>书架还是空的。</p><span>可以导入本地 TXT / EPUB，或从书源搜索并添加书籍。</span><div><button data-testid="local-book-import-empty" class="button secondary" :disabled="localBookImportBusy" @click="importLocalBook">导入本地书</button><button class="button primary" @click="chooseScreen('search')">搜索书籍</button></div></div>
          </section>

          <section v-else-if="screen === 'search'" class="search-view">
            <div class="search-page-heading"><div><h2>发现</h2><span>浏览书源分类、查找书籍和阅读订阅文章</span></div></div>
            <div class="discover-mode-tabs" role="tablist" aria-label="发现方式">
              <button data-testid="discover-mode-discover" role="tab" :aria-selected="discoverMode === 'discover'" :class="{ active: discoverMode === 'discover' }" @click="setDiscoverMode('discover')">书籍发现</button>
              <button data-testid="discover-mode-search" role="tab" :aria-selected="discoverMode === 'search'" :class="{ active: discoverMode === 'search' }" @click="setDiscoverMode('search')">搜索书籍</button>
              <button data-testid="discover-mode-rss" role="tab" :aria-selected="discoverMode === 'rss'" :class="{ active: discoverMode === 'rss' }" @click="setDiscoverMode('rss')">订阅文章</button>
            </div>

            <template v-if="discoverMode !== 'search'">
              <div class="discover-toolbar">
                <label for="discovery-source">选择书源</label>
                <select id="discovery-source" data-testid="discovery-source" v-model="discoverySourceId" :disabled="!enabledSources.length" @change="loadDiscoveryCategories">
                  <option value="" disabled>选择一个已启用的书源</option><option v-for="source in enabledSources" :key="source.id" :value="source.id">{{ source.name }}</option>
                </select>
                <button data-testid="discovery-refresh" class="button secondary" :disabled="discoveryBusy || !discoverySourceId" @click="loadDiscoveryCategories">{{ discoveryBusy ? '正在加载…' : discoverMode === 'rss' ? '刷新订阅' : '加载分类' }}</button>
              </div>
              <div v-if="!enabledSources.length" class="empty-card"><h3>还没有已启用书源</h3><p>启用书源后，可以浏览书籍分类和订阅文章。</p><button class="button secondary" @click="chooseScreen('sources')">管理书源</button></div>
              <div v-else-if="discoveryCategories.length" class="discovery-categories" role="group" aria-label="发现分类">
                <button v-for="category in discoveryCategories" :key="category.categoryId ?? category.title" :data-testid="category.categoryId ? `discovery-category-${category.categoryId}` : undefined" :disabled="!category.categoryId" :class="{ active: discoveryCategoryId === category.categoryId }" @click="category.categoryId && (discoveryCategoryId = category.categoryId, loadDiscoveryPage(category.categoryId, 1))">{{ category.title }}</button>
              </div>
              <div v-if="discoveryBusy && !discoveryResults.length" class="search-loading"><span class="loader-ring"></span><strong>{{ discoverMode === 'rss' ? '正在读取订阅' : '正在加载分类' }}</strong></div>
              <div v-else-if="discoveryError" class="discover-empty" role="status"><p>{{ discoveryError }}</p><button v-if="discoverySourceId" class="button secondary" :disabled="discoveryBusy" @click="loadDiscoveryCategories">重试</button></div>
              <div v-else-if="discoveryResults.length" class="search-results-grid" data-testid="discovery-results">
                <article v-for="result in discoveryResults" :key="result.resultId" :data-testid="`discovery-result-${result.resultId}`" class="result-card">
                  <button class="result-cover" @click="(discoverMode === 'rss' && result.contentSrc) ? openDiscoveryArticle(result) : openSearchResult(result)"><img v-if="result.coverSrc" :src="result.coverSrc" alt="" loading="lazy" /><span v-else class="cover-fallback small-cover"><span>{{ result.sourceName }}</span><strong>{{ result.title }}</strong><small>{{ result.author || '作者未知' }}</small></span></button>
                  <div class="result-details"><span class="source-label">{{ result.sourceName }}</span><button class="result-title" @click="(discoverMode === 'rss' && result.contentSrc) ? openDiscoveryArticle(result) : openSearchResult(result)">{{ result.title }}</button><p class="result-author">{{ result.author || (discoverMode === 'rss' ? '订阅文章' : '作者未知') }}</p><p v-if="result.latestChapter" class="result-latest">{{ result.latestChapter }}</p><div class="result-actions"><button v-if="discoverMode === 'rss' && result.contentSrc" class="button secondary small" :data-testid="`rss-open-${result.resultId}`" @click="openDiscoveryArticle(result)">阅读文章</button><button v-else class="button secondary small" @click="openSearchResult(result)">查看详情</button><button :data-testid="`discovery-add-${result.resultId}`" class="button primary small" :disabled="addBusyResult === result.resultId || discoverMode === 'rss'" @click="addSearchResult(result)">{{ addBusyResult === result.resultId ? '正在添加…' : discoverMode === 'rss' ? '订阅文章' : '＋ 加入书架' }}</button></div></div>
                </article>
              </div>
              <div v-if="discoveryResults.length" class="search-pagination"><button class="button secondary small" :disabled="discoveryPage <= 1 || discoveryBusy" @click="loadDiscoveryPage(discoveryCategoryId, discoveryPage - 1)">← 上一页</button><span>第 {{ discoveryPage }} 页</span><button class="button secondary small" :disabled="!discoveryHasNextPage || discoveryBusy" @click="loadDiscoveryPage(discoveryCategoryId, discoveryPage + 1)">下一页 →</button></div>
            </template>

            <template v-else>
              <form class="search-form" @submit.prevent="runSearch(1)">
                <span class="search-large-icon">⌕</span>
                <input data-testid="search-keyword" v-model="searchKeyword" autofocus placeholder="输入书名、作者或关键词" aria-label="搜索书名、作者或关键词" />
                <button v-if="searchKeyword" type="button" class="clear-search" aria-label="清空" @click="searchKeyword = ''">×</button>
                <button data-testid="search-submit" class="button primary" type="submit" :disabled="searchBusy">{{ searchBusy ? '搜索中…' : '开始搜索' }} <span>→</span></button>
              </form>
              <div class="source-scope-card">
                <div class="scope-heading"><div><strong>搜索范围</strong><small>选择参与搜索的书源</small></div><button class="text-button" @click="setAllSearchSources(!selectedSearchSources)">{{ selectedSearchSources ? '取消全选' : '选择全部' }}</button></div>
                <div v-if="enabledSources.length" class="source-chips">
                  <label v-for="source in enabledSources" :key="source.id" class="source-chip" :class="{ checked: selectedSourceIds.includes(source.id) }"><input type="checkbox" :checked="selectedSourceIds.includes(source.id)" @change="toggleSearchSource(source.id)" /><span class="chip-check">✓</span><span class="chip-label">{{ source.name }}</span><small v-if="source.group">{{ source.group }}</small></label>
                </div>
                <div v-else class="inline-empty">还没有已启用书源。<button class="text-button" @click="chooseScreen('sources')">前往书源管理 →</button></div>
              </div>
              <div v-if="searchWasRun" class="results-section">
                <div class="results-heading"><div><p class="eyebrow">搜索结果</p><h3>{{ searchBusy ? '正在寻找…' : searchKeyword }}</h3></div><span v-if="searchProgress" class="search-status"><i :class="{ spinning: searchBusy }"></i>{{ searchProgress }}</span></div>
                <div v-if="searchBusy && !searchResults.length" class="search-loading"><span class="loader-ring"></span><strong>正在查找相关书籍</strong><p>正在从所选书源寻找匹配的书。</p></div>
                <div v-else-if="searchResults.length" class="search-results-grid">
                  <article v-for="result in searchResults" :key="result.resultId" :data-testid="`search-result-${result.resultId}`" class="result-card">
                    <button class="result-cover" @click="openSearchResult(result)"><img v-if="result.coverSrc" :src="result.coverSrc" alt="" loading="lazy" /><span v-else class="cover-fallback small-cover"><span>{{ result.sourceName }}</span><strong>{{ result.title }}</strong><small>{{ result.author || '作者未知' }}</small></span></button>
                    <div class="result-details"><span class="source-label">{{ result.sourceName }}</span><button class="result-title" @click="openSearchResult(result)">{{ result.title }}</button><p class="result-author">{{ result.author || '作者未知' }}</p><p v-if="result.latestChapter" class="result-latest">最新 · {{ result.latestChapter }}</p><div class="result-actions"><button class="button secondary small" @click="openSearchResult(result)">查看详情</button><button :data-testid="`search-add-${result.resultId}`" class="button primary small" :disabled="addBusyResult === result.resultId" @click="addSearchResult(result)">{{ addBusyResult === result.resultId ? '正在添加…' : '＋ 加入书架' }}</button></div></div>
                  </article>
                </div>
                <div v-else-if="!searchBusy" class="empty-card results-empty"><div class="empty-icon">⌕</div><h3>没有找到匹配的书</h3><p>可以调整关键词或选择其他书源后再试一次。</p></div>
                <div v-if="searchResponseErrors.length || searchResourceErrors.length" class="search-errors"><strong>部分书源没有完成搜索</strong><ul><li v-for="(error, index) in [...searchResponseErrors, ...searchResourceErrors]" :key="`${error.sourceId}-${index}`"><b>{{ error.sourceId ? sourceName(error.sourceId) : '书源' }}</b>：{{ error.message }}</li></ul></div>
                <div v-if="searchResults.length" class="search-pagination"><button class="button secondary small" :disabled="searchPage <= 1 || searchBusy" @click="runSearch(searchPage - 1)">← 上一页</button><span>第 {{ searchPage }} 页</span><button class="button secondary small" :disabled="searchBusy" @click="runSearch(searchPage + 1)">下一页 →</button></div>
              </div>
              <div v-else class="search-hint"><span>⌕</span><p>输入书名、作者，或者你记得的故事关键词。</p></div>
            </template>
          </section>

          <section v-else-if="screen === 'sources'" class="sources-view">
            <div class="source-summary"><div><p class="eyebrow">书源管理</p><h2>把常用的书源整理在这里。</h2><p>按名称和分组管理书源，也可以随时启用或停用。</p></div><div class="source-count"><strong>{{ sources.length }}</strong><span>已导入书源</span><small>{{ enabledSources.length }} 个可用于搜索</small></div></div>
            <div class="source-toolbar"><label class="source-search"><span>⌕</span><input v-model="sourceQuery" placeholder="搜索名称或分组" aria-label="筛选书源" /></label><select v-model="sourceGroupFilter" aria-label="按分组筛选"><option value="">全部分组</option><option v-for="group in sourceGroups" :key="group" :value="group">{{ group }}</option></select><select v-model="sourceFilter" aria-label="按状态筛选"><option value="all">全部状态</option><option value="enabled">已启用</option><option value="disabled">已停用</option></select><button class="button danger-outline small" :disabled="!selectedSourceRows.length" @click="pendingSourceRemoval = [...selectedSourceRows]">移除所选<span v-if="selectedSourceRows.length"> · {{ selectedSourceRows.length }}</span></button></div>
            <div v-if="filteredSources.length" class="source-table-wrap"><div class="source-table-head"><span>书源名称</span><span>分组</span><span>状态</span><span>操作</span></div>
              <article v-for="source in filteredSources" :key="source.id" class="source-row"><label class="source-row-name"><input type="checkbox" :checked="selectedSourceRows.includes(source.id)" @change="toggleSourceRow(source.id)" /><span class="source-monogram">{{ source.name.slice(0, 1) }}</span><span><strong>{{ source.name }}</strong><small>{{ source.id }}</small></span></label><span class="source-group-name">{{ source.group || '未分组' }}</span><label class="source-state-toggle"><input type="checkbox" :checked="source.enabled" :aria-label="`${source.enabled ? '停用' : '启用'}${source.name}`" @change="changeSourceEnabled(source, ($event.target as HTMLInputElement).checked)" /><span class="source-state" :class="{ disabled: !source.enabled }"><i></i>{{ source.enabled ? '已启用' : '已停用' }}</span></label><button class="icon-action" :aria-label="`编辑${source.name}`" title="编辑名称和分组" @click="beginSourceEdit(source)">✎</button></article>
            </div>
            <div v-else class="empty-card source-empty"><div class="empty-icon">◈</div><h3>{{ sources.length ? '没有符合条件的书源' : '这里还没有书源' }}</h3><p>{{ sources.length ? '调整筛选条件，或者清空关键词。' : '导入书源 JSON 后，就可以开始搜索和发现书籍。' }}</p><button class="button primary" @click="sourceImportOpen = true">导入书源</button></div>
            <div class="source-footnote"><span class="info-mark">i</span><p>你可以按需启用来源、整理分组；搜索和阅读时会展示书籍信息与章节内容。</p></div>
          </section>

          <section v-else class="settings-view">
            <div class="settings-intro"><h2>我的</h2><p>阅读记录、阅读偏好、书源和数据管理。</p></div>
            <div class="my-management-row">
              <div><strong>书源管理</strong><span>{{ sources.length }} 个已导入 · {{ enabledSources.length }} 个已启用</span></div>
              <button data-testid="source-manager-open" class="button secondary" @click="chooseScreen('sources')">管理书源</button>
            </div>
            <TaskCenter :tasks="tasks" :book-titles="bookTitleMap" @command="controlTask" />
            <BackupControls :busy="backupBusy" :last-backup-at="lastBackupAt" @create="createBackup" @restore="restoreBackup" />
            <ReadingInsights :days="readingHistory.days" :books="readingHistory.books" :bookmarks="bookmarks" :book-titles="bookTitleMap" :total-duration-ms="readingHistory.totalDurationMs" :total-sessions="readingHistory.totalSessions" @clear-history="clearReadingHistoryData" @delete-book-history="removeBookHistory" @open-bookmark="openBookmark" @delete-bookmark="removeBookmark" />
            <div class="settings-layout">
              <div class="settings-card reader-settings-card">
                <div class="settings-card-heading"><div class="settings-icon">Aa</div><div><h3>阅读显示</h3><p>调整章节的显示效果</p></div></div>
                <div class="setting-control"><div><label for="reader-font-size">字体大小</label><small>当前 {{ settings.reader.fontSizePx }} px</small></div><div class="range-control"><button aria-label="减小字号" @click="updateReaderSetting('fontSizePx', clamp(settings.reader.fontSizePx - 1, 12, 36))">−</button><input id="reader-font-size" type="range" min="12" max="36" :value="settings.reader.fontSizePx" @input="updateReaderSetting('fontSizePx', Number(($event.target as HTMLInputElement).value))" /><button aria-label="增大字号" @click="updateReaderSetting('fontSizePx', clamp(settings.reader.fontSizePx + 1, 12, 36))">＋</button></div></div>
                <div class="setting-control"><div><label for="reader-line-height">行间距</label><small>{{ settings.reader.lineHeight.toFixed(1) }}</small></div><input id="reader-line-height" type="range" min="1.2" max="2.8" step="0.1" :value="settings.reader.lineHeight" @input="updateReaderSetting('lineHeight', Number(($event.target as HTMLInputElement).value))" /></div>
                <div class="setting-control"><div><label for="reader-font-family">正文字体</label><small>只改变显示效果</small></div><select id="reader-font-family" :value="settings.reader.fontFamily" @change="updateReaderSetting('fontFamily', ($event.target as HTMLSelectElement).value)"><option value="serif">衬线字体</option><option value="sans">无衬线字体</option><option value="system">系统字体</option><option value="mono">等宽字体</option></select></div>
                <div class="setting-control"><div><label>阅读主题</label><small>选择喜欢的阅读底色</small></div><div class="theme-options"><button v-for="theme in ([['paper','纸感'],['sepia','护眼'],['dark','夜间'],['system','系统']] as const)" :key="theme[0]" class="theme-option" :class="[`theme-${theme[0]}`, { selected: settings.reader.theme === theme[0] }]" :aria-pressed="settings.reader.theme === theme[0]" @click="setReaderTheme(theme[0])"><span></span>{{ theme[1] }}</button></div></div>
                <div class="setting-control"><div><label for="preload-count">预读章节</label><small>提前准备 {{ settings.reader.preloadCount }} 章</small></div><div class="range-control"><button aria-label="减少预读章节" @click="updateReaderSetting('preloadCount', clamp(settings.reader.preloadCount - 1, 1, 20))">−</button><input id="preload-count" type="range" min="1" max="20" step="1" :value="settings.reader.preloadCount" @input="updateReaderSetting('preloadCount', Number(($event.target as HTMLInputElement).value))" /><button aria-label="增加预读章节" @click="updateReaderSetting('preloadCount', clamp(settings.reader.preloadCount + 1, 1, 20))">＋</button></div></div>
                <div class="settings-save-row"><span>{{ saveSettingsBusy ? '保存中…' : '更改会自动保存。' }}</span><button class="button secondary small" :disabled="saveSettingsBusy" @click="persistSettings">{{ saveSettingsBusy ? '保存中' : '立即保存' }}</button></div>
              </div>
              <div class="settings-card replacement-card"><div class="settings-card-heading"><div class="settings-icon replace-icon">⇄</div><div><h3>显示替换</h3><p>只改变阅读时看到的文字</p></div></div><div v-if="replacementRules.length" class="replacement-list"><div v-for="(rule, index) in replacementRules" :key="rule.id" class="replacement-row replacement-rule-row"><input :aria-label="`第 ${index + 1} 条规则名称`" :value="rule.name" placeholder="规则名称" @input="updateReplacementRule(rule.id, { name: ($event.target as HTMLInputElement).value })" /><input :aria-label="`第 ${index + 1} 条匹配内容`" :value="rule.pattern" placeholder="查找内容" @input="updateReplacementRule(rule.id, { pattern: ($event.target as HTMLInputElement).value })" /><span>→</span><input :aria-label="`第 ${index + 1} 条替换内容`" :value="rule.replacement" placeholder="替换为" @input="updateReplacementRule(rule.id, { replacement: ($event.target as HTMLInputElement).value })" /><label class="rule-check"><input type="checkbox" :checked="rule.enabled" @change="updateReplacementRule(rule.id, { enabled: ($event.target as HTMLInputElement).checked })" />启用</label><label class="rule-check"><input type="checkbox" :checked="rule.isRegex" @change="updateReplacementRule(rule.id, { isRegex: ($event.target as HTMLInputElement).checked })" />正则</label><select :value="rule.scope" :aria-label="`第 ${index + 1} 条规则应用范围`" @change="updateReplacementRule(rule.id, { scope: ($event.target as HTMLSelectElement).value })"><option value="all">全部书籍</option><option v-for="book in shelf.books" :key="book.id" :value="`book:${book.id}`">{{ book.title }}</option></select><button class="icon-action" :aria-label="`删除第 ${index + 1} 条显示替换`" @click="removeReplacement(rule.id)">×</button></div></div><div v-else class="replacement-empty">还没有显示替换规则。</div><button class="button secondary" @click="addReplacement">添加替换规则</button></div>
              <div class="settings-card local-import-card"><div class="settings-card-heading"><div class="settings-icon">↥</div><div><h3>导入本地书</h3><p>支持 TXT 和 EPUB，文件由应用导入并整理到书架。</p></div></div><button class="button secondary" :disabled="localBookImportBusy" @click="importLocalBook">{{ localBookImportBusy ? '正在导入…' : '选择 TXT / EPUB 文件' }}</button></div>
            </div>
          </section>
        </template>
      </div>
    </main>

    <nav v-if="!readerVisible" class="mobile-nav" aria-label="主导航"><button data-testid="nav-home-mobile" :class="{ active: screen === 'home' }" @click="chooseScreen('home')"><span>⌂</span>主页</button><button data-testid="nav-shelf-mobile" :class="{ active: screen === 'shelf' }" @click="chooseScreen('shelf')"><span>▤</span>书架</button><button data-testid="nav-search-mobile" :class="{ active: screen === 'search' }" @click="chooseScreen('search')"><span>⌕</span>发现</button><button data-testid="nav-settings-mobile" :class="{ active: screen === 'settings' || screen === 'sources' }" @click="chooseScreen('settings')"><span>☻</span>我的</button></nav>

    <section v-if="openedBook && !readerVisible" class="overlay-backdrop" @click.self="openedBook = null"><article class="book-detail-panel"><button class="detail-close" aria-label="关闭详情" @click="openedBook = null">×</button><div class="detail-hero"><img v-if="openedBook.coverSrc" :src="openedBook.coverSrc" alt="" /><div v-else class="detail-cover-fallback">{{ openedBook.title.slice(0, 1) }}</div><div><p class="eyebrow">书籍详情</p><h2>{{ openedBook.title }}</h2><p>{{ openedBook.author || '作者未知' }}</p><span class="detail-stat">{{ openedBook.chapterCount || openedBook.chapters.length }} 章 <i></i> {{ openedBook.latestChapter || '目录已同步' }}</span></div></div><div class="detail-actions"><button class="button primary" :disabled="bookPanelBusy || !openedBook.chapterCount" @click="startReading(openedBook)">{{ openedBook.progress?.chapterIndex != null ? '继续阅读' : '开始阅读' }} <span>→</span></button><button class="button secondary" @click="pendingRemoval = { id: openedBook.id, title: openedBook.title }">从书架移除</button></div><BookGroupsEditor :key="openedBook.id" :groups="shelfGroups" :assigned="shelf.books.find((book) => book.id === openedBook?.id)?.groups ?? []" @save="saveBookGroups(openedBook.id, $event)" /><div class="book-task-actions"><button data-testid="book-refresh-catalog" class="button secondary" @click="enqueueBookTask('refresh')">更新目录</button><button data-testid="book-check-new" class="button secondary" @click="enqueueBookTask('check')">检查新章节</button><button data-testid="book-download-chapters" class="button secondary" @click="enqueueBookTask('download')">准备后续章节</button></div><div class="catalog-heading"><div><p class="eyebrow">章节目录</p><h3>章节目录</h3></div><span>{{ openedBook.chapters.length }} / {{ openedBook.chapterCount }} 章</span></div><div class="catalog-list"><button v-for="chapter in openedBook.chapters" :key="chapter.id" class="catalog-row" :class="{ current: chapter.index === openedBook.progress?.chapterIndex }" @click="startReading(openedBook, chapter.index)"><span>{{ String(chapter.index + 1).padStart(2, '0') }}</span><strong>{{ chapter.title }}</strong><small>{{ chapter.src ? '已缓存' : '点击后准备' }}</small><b>›</b></button><div v-if="!openedBook.chapters.length" class="catalog-empty">暂时没有章节目录。</div></div></article></section>

    <section v-if="selectedResult && !readerVisible" class="overlay-backdrop" @click.self="selectedResult = null"><article class="result-detail-panel"><button class="detail-close" aria-label="关闭详情" @click="selectedResult = null">×</button><div class="result-detail-cover"><img v-if="selectedResult.coverSrc" :src="selectedResult.coverSrc" alt="" /><div v-else class="cover-fallback"><span>{{ selectedResult.sourceName }}</span><strong>{{ selectedResult.title }}</strong></div></div><span class="source-label">{{ selectedResult.sourceName }}</span><h2>{{ selectedResult.title }}</h2><p class="result-author">{{ selectedResult.author || '作者未知' }}</p><p class="result-intro">{{ selectedResult.intro || '加入后，可以同步章节目录并开始阅读。' }}</p><div class="detail-actions"><button :data-testid="`search-add-${selectedResult.resultId}`" class="button primary" :disabled="addBusyResult === selectedResult.resultId" @click="addSearchResult(selectedResult)">{{ addBusyResult === selectedResult.resultId ? '正在加入…' : '加入书架' }} <span>→</span></button><button class="button secondary" @click="selectedResult = null">返回结果</button></div></article></section>

    <section v-if="articleOpen" class="overlay-backdrop" @click.self="articleOpen = false"><article class="rss-article-panel"><header><div><span>订阅文章</span><h2>{{ articleTitle }}</h2></div><button class="detail-close" aria-label="关闭文章" @click="articleOpen = false">×</button></header><div v-if="articleBusy" class="reader-loading"><span class="loader-ring"></span><p>正在打开文章…</p></div><iframe v-else data-testid="rss-article-frame" title="订阅文章内容" sandbox="allow-same-origin" :srcdoc="articleHtml"></iframe></article></section>

    <section v-if="readerVisible" class="reader-shell" :class="`reader-theme-${settings.reader.theme}`">
      <header class="reader-topbar"><button data-testid="reader-back" class="reader-back" aria-label="返回目录" @click="closeReader"><span>‹</span><span class="back-label">书籍详情</span></button><div class="reader-book-heading"><strong>{{ readingBook?.title }}</strong><span>{{ currentChapterTitle }}</span></div><div class="reader-header-actions"><span class="read-save-state" :class="progressSaveState">{{ progressSaveState === 'saving' ? '保存中' : progressSaveState === 'saved' ? '已保存' : progressSaveState === 'error' ? '保存失败' : '' }}</span><button data-testid="reader-add-bookmark" class="reader-tool-button bookmark-tool" :aria-label="currentBookmark ? '移除当前书签' : '添加当前页书签'" :title="currentBookmark ? '移除当前书签' : '添加书签'" @click="toggleCurrentBookmark">{{ currentBookmark ? '★' : '☆' }}</button><button class="reader-tool-button" aria-label="阅读显示设置" @click="readerControlsOpen = !readerControlsOpen">Aa</button><button class="reader-tool-button" aria-label="打开目录" @click="closeReader">☷</button></div></header>
      <div class="reader-main"><div class="reader-chapter-heading"><span class="reader-book-label">{{ readingBook?.title }}</span><h1 data-testid="reader-chapter-title">{{ currentChapterTitle }}</h1><span class="chapter-rule"></span></div><div class="reader-content-frame"><div v-if="readerBusy" class="reader-loading"><span class="loader-ring"></span><p>{{ prefetchBusy ? '正在准备后续章节…' : '正在打开章节…' }}</p></div><iframe v-else ref="readerFrame" data-testid="reader-frame" class="chapter-frame" title="章节内容" sandbox="allow-same-origin" :srcdoc="readingChapterHtml" @load="measureReaderPages()"></iframe></div><div class="reader-edge reader-edge-left" @click="turnPage(-1)" aria-hidden="true"></div><div class="reader-edge reader-edge-right" @click="turnPage(1)" aria-hidden="true"></div></div>
      <footer class="reader-footer"><button data-testid="reader-prev-chapter" class="chapter-turn" :disabled="readingChapterIndex <= 0 || readerBusy" @click="changeChapter(-1)">上一章</button><button data-testid="reader-prev" class="turn-button" :disabled="readerBusy" aria-label="上一页" @click="turnPage(-1)"><span>‹</span><small>上一页</small></button><div class="reader-page-status"><div class="reader-progress-track"><span :style="{ width: `${readPercent}%` }"></span></div><span data-testid="reader-page-indicator">{{ readerPageIndex + 1 }} / {{ readerPageCount }}</span><span class="reader-footer-divider"></span><span>第 {{ readingChapterIndex + 1 }} / {{ readingBook?.chapterCount || readingBook?.chapters.length || '—' }} 章</span><span v-if="prefetchBusy" class="preload-indicator">正在准备</span></div><button data-testid="reader-next" class="turn-button next" :disabled="readerBusy" aria-label="下一页" @click="turnPage(1)"><small>下一页</small><span>›</span></button><button data-testid="reader-next-chapter" class="chapter-turn" :disabled="readingChapterIndex + 1 >= (readingBook?.chapterCount || 0) || readerBusy" @click="changeChapter(1)">下一章</button></footer>
      <aside v-if="readerControlsOpen" data-testid="reader-settings-popover" class="reader-settings-popover"><div class="popover-title"><span>阅读显示</span><button aria-label="关闭阅读设置" @click="readerControlsOpen = false">×</button></div><div class="setting-control compact-setting"><div><label>字号</label><small data-testid="reader-font-size-value">{{ settings.reader.fontSizePx }} px</small></div><div class="range-control"><button data-testid="reader-font-decrease" aria-label="减小字号" @click="updateReaderSetting('fontSizePx', clamp(settings.reader.fontSizePx - 1, 12, 36))">−</button><input type="range" min="12" max="36" :value="settings.reader.fontSizePx" @input="updateReaderSetting('fontSizePx', Number(($event.target as HTMLInputElement).value))" /><button data-testid="reader-font-increase" aria-label="增大字号" @click="updateReaderSetting('fontSizePx', clamp(settings.reader.fontSizePx + 1, 12, 36))">＋</button></div></div><div class="setting-control compact-setting"><div><label>行间距</label><small>{{ settings.reader.lineHeight.toFixed(1) }}</small></div><input type="range" min="1.2" max="2.8" step="0.1" :value="settings.reader.lineHeight" @input="updateReaderSetting('lineHeight', Number(($event.target as HTMLInputElement).value))" /></div><div class="setting-control compact-setting"><div><label>阅读主题</label></div><div class="theme-options"><button v-for="theme in ([['paper','纸'],['sepia','暖'],['dark','夜'],['system','白']] as const)" :key="theme[0]" class="theme-option compact-theme" :class="[`theme-${theme[0]}`, { selected: settings.reader.theme === theme[0] }]" @click="setReaderTheme(theme[0])">{{ theme[1] }}</button></div></div><button class="text-button popover-save" :disabled="saveSettingsBusy" @click="persistSettings">{{ saveSettingsBusy ? '保存中…' : '保存阅读设置' }}</button></aside>
    </section>

    <section v-if="bookmarkEditorOpen" class="modal-backdrop" @click.self="bookmarkEditorOpen = false"><article class="app-modal bookmark-modal"><button class="detail-close" aria-label="关闭书签窗口" @click="bookmarkEditorOpen = false">×</button><p class="eyebrow">书签</p><h2>为这一页添加备注</h2><p class="modal-description">{{ readingBook?.title }} · {{ currentChapterTitle }} · 第 {{ readerPageIndex + 1 }} 页</p><textarea v-model="bookmarkNote" aria-label="书签备注" maxlength="300" placeholder="写下想记住的内容（可选）"></textarea><div class="modal-actions"><button class="button secondary" @click="bookmarkEditorOpen = false">取消</button><button class="button primary" :disabled="bookmarkBusy" @click="addBookmark">{{ bookmarkBusy ? '保存中…' : '保存书签' }}</button></div></article></section>

    <section v-if="sourceImportOpen" class="modal-backdrop" @click.self="sourceImportOpen = false"><article class="app-modal source-import-modal"><button class="detail-close" aria-label="关闭导入窗口" @click="sourceImportOpen = false">×</button><p class="eyebrow">导入书源</p><h2>添加书源</h2><p class="modal-description">选择书源文件即可导入；书源规则会由应用安全处理，不会显示在页面上。</p><div class="file-drop"><span class="file-icon">↥</span><strong>从设备选择书源文件</strong><small>支持 JSON 格式，文件由应用直接读取。</small><button data-testid="source-import-picker" class="button secondary" :disabled="sourceImportBusy" @click="importSourceFileFromPicker">{{ sourceImportBusy ? '正在导入…' : '选择 JSON 文件' }}</button></div><div class="modal-actions"><button class="button secondary" @click="sourceImportOpen = false">取消</button></div></article></section>

    <section v-if="editingSource" class="modal-backdrop" @click.self="editingSource = null"><article class="app-modal source-edit-modal"><button class="detail-close" aria-label="关闭书源编辑" @click="editingSource = null">×</button><p class="eyebrow">书源信息</p><h2>编辑书源</h2><p class="modal-description">修改书源的名称与分组，方便查找和管理。</p><label class="form-field"><span>显示名称</span><input v-model="sourceEditName" autofocus maxlength="100" /></label><label class="form-field"><span>分组</span><input v-model="sourceEditGroup" maxlength="100" placeholder="留空表示未分组" /></label><div class="modal-actions"><button class="button secondary" @click="editingSource = null">取消</button><button class="button primary" @click="saveSourceMetadata">保存</button></div></article></section>

    <section v-if="pendingRemoval" class="modal-backdrop" @click.self="pendingRemoval = null"><article class="app-modal confirm-modal"><div class="confirm-symbol">⌫</div><h2>从书架移除这本书？</h2><p>《{{ pendingRemoval.title }}》会从你的书架中移除。</p><div class="modal-actions"><button class="button secondary" @click="pendingRemoval = null">保留</button><button class="button danger" @click="removeBookFromShelf">确认移除</button></div></article></section>

    <section v-if="pendingSourceRemoval.length" class="modal-backdrop" @click.self="pendingSourceRemoval = []"><article class="app-modal confirm-modal"><div class="confirm-symbol source-confirm">◈</div><h2>移除这些书源？</h2><p>已添加到书架的书籍不会受到影响。</p><div class="modal-actions"><button class="button secondary" @click="pendingSourceRemoval = []">取消</button><button class="button danger" @click="deleteSelectedSources">确认移除</button></div></article></section>

    <transition name="toast"><div v-if="toast" class="toast-message" :class="toastKind" role="status"><span>{{ toastKind === 'success' ? '✓' : '!' }}</span>{{ toast }}</div></transition>
  </div>
</template>

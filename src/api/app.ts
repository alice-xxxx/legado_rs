import { invoke, isTauri } from "@tauri-apps/api/core";

/** A runtime URL that Rust materializes for the current process. */
export interface ResourceDescriptor {
  resourceId: string;
  src: string;
  contentType: string;
}

export interface SourceMetadata {
  id: string;
  name: string;
  group?: string;
  enabled: boolean;
  isRss?: boolean;
}

export interface ChapterResource {
  id: string;
  title: string;
  index: number;
  src?: string;
}

export interface BookResource {
  schemaVersion: number;
  id: string;
  title: string;
  author?: string;
  intro?: string;
  kind?: string;
  wordCount?: string;
  sourceId?: string;
  sourceName?: string;
  sourceGroup?: string;
  canChangeSource?: boolean;
  coverSrc?: string;
  chapterCount: number;
  latestChapter?: string;
  progress?: {
    chapterId?: string | null;
    chapterIndex?: number;
    offset?: number;
    updatedAtMs?: number;
  };
  chapters: ChapterResource[];
}

export interface ShelfResource {
  schemaVersion: number;
  sort?: ShelfSortKey;
  sortOrder?: ShelfSortOrder;
  groups?: string[];
  books: Array<Pick<BookResource, "id" | "title" | "author" | "coverSrc" | "chapterCount" | "latestChapter" | "progress"> & { groups?: string[] }>;
}

export type ShelfSortKey = "updatedAt" | "title" | "author" | "latestChapter" | "progress" | "custom";
export type ShelfSortOrder = "ascending" | "descending";

export interface Bookmark {
  id: string;
  bookId: string;
  chapterIndex: number;
  chapterTitle?: string;
  offset: number;
  note: string;
  orphaned?: boolean;
  createdAtMs: number;
  updatedAtMs: number;
}

export interface BookmarksResource {
  schemaVersion: number;
  bookmarks: Bookmark[];
}

export interface ReadingHistoryResource {
  schemaVersion: number;
  sessions: Array<{ bookId: string; sessionId: string; durationMs: number; endedAtMs: number; utcOffsetMinutes: number }>;
  books: Array<{ bookId: string; sessionCount: number; totalDurationMs: number; lastReadAtMs: number }>;
  days: Array<{ day: string; sessionCount: number; totalDurationMs: number; books: Array<{ bookId: string; sessionCount: number; totalDurationMs: number }> }>;
  totalDurationMs: number;
  totalSessions: number;
}

export interface DisplayReplacementRule {
  id: string;
  name: string;
  pattern: string;
  replacement: string;
  enabled: boolean;
  isRegex: boolean;
  scope: string;
}

export interface ReplacementRulesResource {
  schemaVersion: number;
  rules: DisplayReplacementRule[];
}

export interface SearchBookResult {
  resultId: string;
  sourceId: string;
  sourceName: string;
  articleId?: string;
  contentSrc?: string;
  isRead?: boolean;
  isFavorite?: boolean;
  title: string;
  author?: string;
  coverSrc?: string;
  bookUrl?: string;
  intro?: string;
  latestChapter?: string;
  requiresIdentityConfirmation?: boolean;
  [key: string]: unknown;
}

export interface SearchResource {
  schemaVersion: number;
  keyword: string;
  page: number;
  results: SearchBookResult[];
  errors: Array<{ sourceId?: string; message: string }>;
  complete?: boolean;
}

export interface SearchHistoryEntry {
  query: string;
  normalizedQuery: string;
  usage: number;
  firstUseTimeMs: number;
  lastUseTimeMs: number;
}

export interface SearchHistoryResource {
  schemaVersion: number;
  entries: SearchHistoryEntry[];
  processedSearchIds: string[];
}

export interface ReaderSettings {
  fontSizePx: number;
  lineHeight: number;
  fontFamily: string;
  textColor: string;
  backgroundColor: string;
  textAlign: string;
  theme: string;
  preloadCount: number;
  replacements: Array<{ find: string; replace: string }>;
}

export interface ReadingProgress {
  chapterId: string | null;
  chapterIndex: number;
  offset: number;
  updatedAtMs: number;
}

export interface SourcePatch {
  name?: string;
  group?: string | null;
  enabled?: boolean;
}

export interface AppSettingsResource {
  schemaVersion: number;
  reader: ReaderSettings;
}

export interface AppBootstrap {
  shelf: ResourceDescriptor;
  settings: ResourceDescriptor;
  searchHistory: ResourceDescriptor;
  sources: SourceMetadata[];
}

export interface SearchResponse {
  resource: ResourceDescriptor;
  bookCount: number;
  errors: Array<{ sourceId?: string; message: string }>;
}

export interface BookMutationResponse {
  book: ResourceDescriptor;
  shelf: ResourceDescriptor;
}

export interface BookSourceMutationResponse extends BookMutationResponse {
  progress: ReadingProgress;
  movedProgress: boolean;
  bookmarks: {
    resource: ResourceDescriptor;
    migratedCount: number;
    orphanedCount: number;
  };
}

export interface SourceImportResponse {
  cancelled?: boolean;
  sources: SourceMetadata[];
}

export interface LocalBookImportResponse {
  cancelled?: boolean;
  passwordRequired?: boolean;
  importToken?: string;
  book?: ResourceDescriptor;
  shelf?: ResourceDescriptor;
}

export interface LocalBookImportOptions {
  charset?: string | null;
  tocRegex?: string | null;
}

export interface AppTask {
  id: string;
  kind: string;
  status: string;
  searchId?: string;
  bookId?: string;
  sourceIds?: string[];
  keyword?: string;
  page: number;
  fromIndex: number;
  total: number;
  completed: number;
  createdAtMs: number;
  updatedAtMs: number;
  error?: string;
  result?: {
    bookResourceId?: string;
    addedCount?: number;
    movedProgress?: boolean;
    committed?: boolean;
  };
}

export interface TasksResource {
  schemaVersion: number;
  tasks: AppTask[];
}

export interface TaskResponse {
  taskId?: string;
  task: AppTask;
  resource: ResourceDescriptor;
}

export interface DiscoveryCategory {
  categoryId?: string;
  title: string;
  kind?: string;
  style?: { cols?: number; rows?: number; layoutFlexBasisPercent?: number };
}

export interface DiscoveryCategoriesResource {
  schemaVersion: number;
  sourceId: string;
  categories: DiscoveryCategory[];
}

export interface DiscoveryFavorite {
  sourceId: string;
  categoryId: string;
  title: string;
  sourceName: string;
  kind?: string;
  style?: DiscoveryCategory["style"];
  position: number;
  createdAtMs: number;
}

export interface DiscoveryFavoritesResource {
  schemaVersion: number;
  favorites: DiscoveryFavorite[];
}

export interface DiscoveryResponse {
  sourceId: string;
  categoryId?: string;
  categoryCount?: number;
  page?: number;
  hasNextPage?: boolean;
  bookCount?: number;
  errors?: Array<{ sourceId?: string; message: string }>;
  resource: ResourceDescriptor;
  rssState?: ResourceDescriptor;
}

export interface BackupResponse {
  cancelled?: boolean;
  backup?: boolean;
  restored?: boolean;
  bootstrap?: AppBootstrap;
}

export interface HomeSection {
  id: string;
  title: string;
  sourceId: string;
  sourceName: string;
  categoryId: string;
  categoryName: string;
  style: 0 | 1 | 2 | 3;
  sortOrder: number;
  coverVideo?: boolean;
}

export interface HomeTab {
  id: string;
  title: string;
  sortOrder: number;
  sections: HomeSection[];
}

export interface HomeConfigDocument {
  schemaVersion: number;
  tabs: HomeTab[];
}

export type RssFilter = "all" | "unread" | "read" | "favorites";

export interface RssSubscriptionState {
  sourceId: string;
  filter: RssFilter;
}

export interface RssArticleState {
  sourceId: string;
  articleId: string;
  isRead: boolean;
  isFavorite: boolean;
  updatedAtMs: number;
}

export interface RssStateDocument {
  schemaVersion: number;
  subscriptions: RssSubscriptionState[];
  articles: RssArticleState[];
}

function requireTauri(): void {
  if (!isTauri()) {
    throw new Error("此功能需要在应用中使用。");
  }
}

async function command<T>(name: string, args?: Record<string, unknown>): Promise<T> {
  requireTauri();
  return invoke<T>(name, args);
}

export async function readResource<T>(descriptor: ResourceDescriptor | string): Promise<T> {
  const src = typeof descriptor === "string" ? descriptor : descriptor.src;
  if (!src) throw new Error("资源暂时不可用。");
  let response: Response;
  try {
    response = await fetch(src, { cache: "no-store" });
  } catch (error) {
    throw new Error(`无法读取内容：${error instanceof Error ? error.message : String(error)}`);
  }
  if (!response.ok) {
    throw new Error(`读取资源失败（HTTP ${response.status}）：${src}`);
  }
  return (await response.json()) as T;
}

export async function readTextResource(descriptor: ResourceDescriptor | string): Promise<string> {
  const src = typeof descriptor === "string" ? descriptor : descriptor.src;
  if (!src) throw new Error("章节内容暂时不可用。");
  let response: Response;
  try {
    response = await fetch(src, { cache: "no-store" });
  } catch (error) {
    throw new Error(`无法读取章节资源：${error instanceof Error ? error.message : String(error)}`);
  }
  if (!response.ok) throw new ResourceHttpError(response.status, `读取章节失败（HTTP ${response.status}）。`);
  return response.text();
}

export class ResourceHttpError extends Error {
  constructor(readonly status: number, message: string) {
    super(message);
    this.name = "ResourceHttpError";
  }
}

export const appBootstrap = () => command<AppBootstrap>("app_bootstrap");
export const getSearchHistory = () => command<ResourceDescriptor>("get_search_history");
export const deleteSearchHistory = (query: string) =>
  command<ResourceDescriptor>("delete_search_history", { query });
export const clearSearchHistory = () => command<ResourceDescriptor>("clear_search_history");
export const listSources = () => command<{ sources: SourceMetadata[] }>("list_sources");
export const importSources = (sourceJson: string) =>
  command<{ sources: SourceMetadata[] }>("import_sources", { sourceJson });
export const importSourcesFromPicker = () =>
  command<SourceImportResponse>("import_sources_from_picker");
export const importBookFromPicker = (options: LocalBookImportOptions = {}) =>
  command<LocalBookImportResponse>("import_book_from_picker", { options });
export const importProtectedPdf = (importToken: string, password: string) =>
  command<LocalBookImportResponse>("import_protected_pdf", { importToken, password });
export const cancelPendingPdfImport = (importToken: string) =>
  command<{ cancelled: boolean }>("cancel_pending_pdf_import", { importToken });
export const removeSources = (sourceIds: string[]) =>
  command<{ sources: SourceMetadata[] }>("remove_sources", { sourceIds });
export const updateSource = (sourceId: string, patch: SourcePatch) =>
  command<{ sources: SourceMetadata[] }>("update_source", { sourceId, patch });
export const searchBooks = (sourceIds: string[], keyword: string, page = 1) =>
  command<SearchResponse>("search_books", { sourceIds, keyword, page });
export const startSearch = (sourceIds: string[], keyword: string, page = 1) =>
  command<TaskResponse & { resource: ResourceDescriptor }>("start_search", { sourceIds, keyword, page });
export const searchBookSourceCandidates = (bookId: string, sourceIds: string[], keyword?: string, page = 1) =>
  command<TaskResponse>("search_book_source_candidates", { bookId, sourceIds, keyword, page });
export const changeBookSource = (bookId: string, resultId: string, confirmMissingAuthor = false) =>
  command<BookSourceMutationResponse>("change_book_source", { bookId, resultId, confirmMissingAuthor });
export const tasksResource = () => command<{ resource: ResourceDescriptor }>("tasks_resource");
export const startChapterDownload = (bookId: string, fromIndex: number, count: number) =>
  command<TaskResponse>("start_chapter_download", { bookId, fromIndex, count });
export const refreshChapters = (bookId: string) => command<TaskResponse>("refresh_chapters", { bookId });
export const checkNewChapters = (bookId: string) => command<TaskResponse>("check_new_chapters", { bookId });
export const pauseTask = (taskId: string) => command<{ task: AppTask; resource: ResourceDescriptor }>("pause_task", { taskId });
export const resumeTask = (taskId: string) => command<{ task: AppTask; resource: ResourceDescriptor }>("resume_task", { taskId });
export const cancelTask = (taskId: string) => command<{ task: AppTask; resource: ResourceDescriptor }>("cancel_task", { taskId });
export const addBook = (resultId: string) =>
  command<BookMutationResponse>("add_book", { resultId });
export const getBook = (bookId: string) => command<ResourceDescriptor>("get_book", { bookId });
export const prepareChapters = (bookId: string, fromIndex: number, count: number) =>
  command<{ book: ResourceDescriptor; prepared: number }>("prepare_chapters", {
    bookId,
    fromIndex,
    count,
  });
export const removeBook = (bookId: string) =>
  command<{ shelf: ResourceDescriptor }>("remove_book", { bookId });
export const saveProgress = (bookId: string, progress: ReadingProgress) =>
  command<ResourceDescriptor>("save_progress", { bookId, progress });
export const saveSettings = (settings: AppSettingsResource) =>
  command<ResourceDescriptor>("save_settings", { settings });
export const listBookmarks = () => command<ResourceDescriptor>("list_bookmarks");
export const upsertBookmark = (input: Omit<Bookmark, "id" | "createdAtMs" | "updatedAtMs"> & { id?: string }) =>
  command<ResourceDescriptor>("upsert_bookmark", { input });
export const deleteBookmark = (bookmarkId: string) =>
  command<ResourceDescriptor>("delete_bookmark", { bookmarkId });
export const readingHistoryResource = () => command<ResourceDescriptor>("reading_history_resource");
export const recordReadingSession = (session: ReadingHistoryResource["sessions"][number]) =>
  command<ResourceDescriptor>("record_reading_session", { session });
export const deleteReadingHistoryForBook = (bookId: string) =>
  command<ResourceDescriptor>("delete_reading_history_for_book", { bookId });
export const clearReadingHistory = () => command<ResourceDescriptor>("clear_reading_history");
export const listReplacementRules = () => command<ResourceDescriptor>("list_replacement_rules");
export const upsertReplacementRule = (input: Omit<DisplayReplacementRule, "id"> & { id?: string }) =>
  command<ResourceDescriptor>("upsert_replacement_rule", { input });
export const deleteReplacementRule = (ruleId: string) =>
  command<ResourceDescriptor>("delete_replacement_rule", { ruleId });
export const setBookGroups = (bookId: string, groups: string[]) =>
  command<ResourceDescriptor>("set_book_groups", { bookId, groups });
export const renameShelfGroup = (oldName: string, newName: string) =>
  command<ResourceDescriptor>("rename_shelf_group", { oldName, newName });
export const deleteShelfGroup = (groupName: string) =>
  command<ResourceDescriptor>("delete_shelf_group", { groupName });
export const setShelfSort = (key: ShelfSortKey, order: ShelfSortOrder) =>
  command<ResourceDescriptor>("set_shelf_sort", { key, order });
export const setShelfOrder = (orderedBookIds: string[]) =>
  command<ResourceDescriptor>("set_shelf_order", { orderedBookIds });
export const createShelfGroup = (groupName: string) =>
  command<ResourceDescriptor>("create_shelf_group", { groupName });
export const listDiscoveryCategories = (sourceId: string) =>
  command<DiscoveryResponse>("list_discovery_categories", { sourceId });
export const listDiscoveryFavorites = () =>
  command<{ resource: ResourceDescriptor }>("list_discovery_favorites");
export const setDiscoveryFavorite = (sourceId: string, categoryId: string, favorite: boolean) =>
  command<ResourceDescriptor>("set_discovery_favorite", { sourceId, categoryId, favorite });
export const listDiscoveryBooks = (sourceId: string, categoryId: string, page = 1) =>
  command<DiscoveryResponse>("list_discovery_books", { sourceId, categoryId, page });
export const listRssCategories = (sourceId: string) =>
  command<DiscoveryResponse>("list_rss_categories", { sourceId });
export const listRssArticles = (sourceId: string, categoryId: string, page = 1) =>
  command<DiscoveryResponse>("list_rss_articles", { sourceId, categoryId, page });
export const openRssArticle = (sourceId: string, articleId: string) =>
  command<{ sourceId: string; articleId: string; resource: ResourceDescriptor }>("open_rss_article", { sourceId, articleId });
export const createBackupFromPicker = () => command<BackupResponse>("create_backup_from_picker");
export const restoreBackupFromPicker = () => command<BackupResponse>("restore_backup_from_picker");
export const getHomeConfig = () => command<ResourceDescriptor>("get_home_config");
export const saveHomeConfig = (config: HomeConfigDocument) =>
  command<ResourceDescriptor>("save_home_config", { config });
export const getRssState = () => command<ResourceDescriptor>("get_rss_state");
export const setRssFilter = (sourceId: string, filter: RssFilter) =>
  command<ResourceDescriptor>("set_rss_filter", { sourceId, filter });
export const setRssArticleState = (
  sourceId: string,
  articleId: string,
  patch: { isRead?: boolean; isFavorite?: boolean },
) => command<ResourceDescriptor>("set_rss_article_state", { sourceId, articleId, ...patch });
export const unsubscribeRss = (sourceId: string) =>
  command<{ sources: SourceMetadata[]; resource: ResourceDescriptor }>("unsubscribe_rss", { sourceId });

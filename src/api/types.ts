export type ResourceFormat =
  | "json"
  | "html"
  | "text"
  | "stylesheet"
  | "image"
  | "font"
  | "audio"
  | "video"
  | "pdf"
  | "binary";

/** A runtime resource description produced by Rust for WebView consumption. */
export interface ResourceDescriptor {
  resourceId: string;
  src: string;
  contentType: string;
  format: ResourceFormat;
}

export interface SourceMetadata {
  id: string;
  name: string;
  group?: string;
  enabled: boolean;
  isRss?: boolean;
  mediaType?: "audio" | "video";
  capabilities?: { search: boolean; detail: boolean; toc: boolean; content: boolean };
  userAgentOverride?: string | null;
}

export interface SourceDefinitionEntry extends SourceMetadata {
  definitionJson: string;
}

export interface SourceDefinitionsResource {
  schemaVersion: number;
  sources: SourceDefinitionEntry[];
}

export interface SourceLoginField {
  name: string;
  label: string;
  type: "text" | "password" | "select" | "toggle";
  password: boolean;
  choices?: string[];
}

export interface SourceLoginAction {
  id: number;
  label: string;
}

export type SourceLoginForm = {
  mode: "form";
  canLogin: boolean;
  fields: SourceLoginField[];
  actions: SourceLoginAction[];
} | {
  mode: "web";
  message: string;
} | {
  mode: "webUnavailable";
  message: string;
};

export interface SourceLoginResult {
  status: "executed" | "failed";
  code: string;
  message: string;
  authenticated: null;
}

export interface SourceWebLoginResult {
  status: "executed";
  importedCount: number;
  message: string;
}

export type ChapterContentFormat = "text" | "richText" | "audio" | "video" | "pdf";

export interface ChapterResourceDescriptor extends ResourceDescriptor {
  /** Semantic chapter kind supplied by Rust; do not infer it from URL or MIME type. */
  contentFormat: ChapterContentFormat;
}

export interface ChapterResource {
  id: string;
  title: string;
  index: number;
  /** The sole location and semantic format for a prepared chapter. */
  resource?: ChapterResourceDescriptor;
}

export interface ChapterTextResource {
  schemaVersion: number;
  kind: "text";
  text: string;
}

export interface ChapterRichTextResource {
  schemaVersion: number;
  kind: "richText";
  markup: string;
}

export interface ChapterMediaSource {
  src?: string | null;
  label: string;
  format?: "direct" | "hls" | null;
  selected: boolean;
  unavailableReason?: string | null;
}

export interface ChapterMediaResource {
  schemaVersion: number;
  kind: "media";
  mediaType: "audio" | "video";
  src: string;
  format: "direct" | "hls";
  sources: ChapterMediaSource[];
}

export interface ChapterPdfPageResource {
  schemaVersion: number;
  kind: "pdfPage";
  src: string;
  pageIndex: number;
  defaultZoom: "page-fit" | "page-width" | "actual-size";
}

export interface BookResource {
  schemaVersion: number;
  id: string;
  title: string;
  author?: string;
  intro?: string;
  kind?: string;
  /** Audio and video sources store progress.offset in milliseconds. */
  mediaType?: "audio" | "video";
  wordCount?: string;
  sourceId?: string;
  sourceName?: string;
  sourceGroup?: string;
  displayBase?: BookDisplayMetadataValues;
  displayOverrides?: Partial<Record<keyof BookDisplayMetadataValues, string>>;
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

export interface BookDisplayMetadataValues {
  title: string;
  author?: string;
  intro?: string;
  coverSrc?: string;
}

export interface BookDisplayMetadataPatch {
  title?: string | null;
  author?: string | null;
  intro?: string | null;
  coverSrc?: string | null;
}

export type BookCoverPickerResponse =
  | { cancelled: true }
  | { cancelled: false; assetId: string; coverSrc: string; previewSrc: string };

export type ReaderBackgroundPickerResponse =
  | { cancelled: true }
  | { cancelled: false; settings: ResourceDescriptor };

export type BookDisplayMetadataUpdateResponse =
  | {
      commitState: "committed";
      recoveryRequired: boolean;
      warning: string | null;
      book: ResourceDescriptor;
      shelf: ResourceDescriptor;
    }
  | {
      commitState: "notCommitted" | "indeterminate";
      recoveryRequired: boolean;
      warning: string | null;
      book?: never;
      shelf?: never;
    };

export interface ShelfResource {
  schemaVersion: number;
  sort?: ShelfSortKey;
  sortOrder?: ShelfSortOrder;
  groups?: string[];
  books: Array<Pick<BookResource, "id" | "title" | "author" | "coverSrc" | "chapterCount" | "latestChapter" | "progress" | "kind" | "mediaType"> & { bookSrc: string; groups?: string[] }>;
}

export interface BatchRemoveBookError {
  bookId: string;
  outcome: "notCommitted" | "unknown";
  commitState: "notCommitted" | "indeterminate";
  recoveryRequired: boolean;
  message: string;
}

export interface BatchRemoveBookWarning {
  bookId: string;
  outcome: "committed";
  commitState?: "committed";
  recoveryRequired: boolean;
  message: string;
}

export interface BatchRemoveBooksResponse {
  shelf: ResourceDescriptor;
  completedIds: string[];
  errors: BatchRemoveBookError[];
  warnings: BatchRemoveBookWarning[];
  recoveryRequired: boolean;
  notAttemptedIds: string[];
}

export type RemoveBookResponse =
  | {
      outcome: "deleted";
      commitState: "committed";
      bookId: string;
      shelf: ResourceDescriptor;
      recoveryRequired: boolean;
      warning: string | null;
    }
  | {
      commitState: "notCommitted" | "indeterminate";
      recoveryRequired: boolean;
      error: string;
      outcome?: never;
      bookId?: never;
      shelf?: never;
      warning?: never;
    };

export type ResetBookProgressResponse =
  | {
      commitState: "committed";
      recoveryRequired: boolean;
      warning: string | null;
      book: ResourceDescriptor;
      shelf: ResourceDescriptor;
    }
  | {
      commitState: "notCommitted" | "indeterminate";
      recoveryRequired: boolean;
      warning: string | null;
      book?: never;
      shelf?: never;
    };

export type ClearBookChapterCacheResponse =
  | {
      commitState: "committed";
      recoveryRequired: boolean;
      warning: string | null;
      clearedCount: number;
      book: ResourceDescriptor;
    }
  | {
      commitState: "notCommitted";
      recoveryRequired: boolean;
      warning: string | null;
      error?: string;
      clearedCount: number;
      book?: never;
    }
  | {
      commitState: "indeterminate";
      recoveryRequired: boolean;
      warning: string | null;
      clearedCount: number;
      book?: never;
    };

export interface ChapterCacheUsage {
  chapterResourceCount: number;
  chapterResourceBytes: number;
  onlineChapterResourceCount: number;
  onlineChapterResourceBytes: number;
  localChapterResourceCount: number;
  localChapterResourceBytes: number;
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

export type ReadingStatisticsResource = Pick<ReadingHistoryResource, "books" | "days" | "totalDurationMs" | "totalSessions">;

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

export type ReplacementRulesMutation =
  | { kind: "local" }
  | { kind: "url"; url: string }
  | { kind: "save"; id: string | null; json: string }
  | { kind: "delete"; ruleIds: string[] };

export type ReplacementRulesMutationResponse =
  | { cancelled: true }
  | { resource: ResourceDescriptor; importedCount: number }
  | { resource: ResourceDescriptor; savedId: string }
  | { resource: ResourceDescriptor };

/** Rust 处理后的单个书源候选；resultId 用于详情、入架和换源操作。 */
export interface SearchResultSource {
  resultId: string;
  sourceId: string;
  sourceName: string;
  mediaType?: "audio" | "video";
  kind?: string;
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

/** Rust 已合并的书目。顶层字段是默认候选，sources 保留全部来源。 */
export interface SearchBookResult extends SearchResultSource {
  sources?: SearchResultSource[];
}

export interface SearchResource {
  schemaVersion: number;
  searchId?: string;
  taskId?: string;
  keyword: string;
  sourceIds?: string[];
  page: number;
  results: SearchBookResult[];
  errors: Array<{ sourceId?: string; message: string }>;
  status?: "queued" | "running" | "paused" | "completed" | "cancelled" | "failed";
  completedSources?: number;
  totalSources?: number;
  complete?: boolean;
  cancelled?: boolean;
  error?: string;
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
  /** Runtime URL materialized by the resource server from a stable resource ref. */
  backgroundImageSrc?: string;
  /** Visible opacity of the reader background image, from 0 to 100 percent. */
  backgroundImageOpacity: number;
  textAlign: string;
  theme: string;
  preloadCount: number;
  verticalScroll: boolean;
  audioPlaybackRate: number;
  /** Default forward/backward skip in seconds for audio media controls. */
  audioSkipSeconds: number;
  /** Default video playback rate, persisted separately from audio speed. */
  videoPlaybackRate: number;
  ttsEngine: "system" | "http";
  ttsRate: number;
  ttsVoiceURI: string;
  /** User-selected HTTP TTS configuration ID. */
  httpTtsConfigId?: string | null;
  ttsContinueAcrossChapters: boolean;
  ttsStartPosition: "current" | "chapter";
  replacements: Array<{ find: string; replace: string }>;
}

export interface HttpTtsConfigMetadata {
  id: string;
  name: string;
  enabled: boolean;
}

export interface HttpTtsConfigEntry {
  id: string;
  name: string;
  enabled: boolean;
  config: Record<string, unknown>;
}

export interface HttpTtsConfigResource {
  schemaVersion: number;
  configs: HttpTtsConfigEntry[];
}

export interface TxtTocRule {
  id: string;
  name: string;
  rule: string;
  example?: string;
  serialNumber: number;
  enable: boolean;
}

export interface TxtTocRulesDocument {
  schemaVersion: number;
  rules: TxtTocRule[];
}

export type TxtTocRulesMutation =
  | { kind: "local" }
  | { kind: "url"; url: string }
  | { kind: "save"; id: string | null; json: string }
  | { kind: "reorder"; orderedIds: string[] }
  | { kind: "delete"; ruleIds: string[] };

export type TxtTocRulesMutationResponse =
  | { cancelled: true }
  | { resource: ResourceDescriptor; importedCount: number }
  | { resource: ResourceDescriptor; savedId: string }
  | { resource: ResourceDescriptor };

/** Vue-local form state; the Tauri save command receives the completed JSON string. */
export interface HttpTtsConfigEditorFields {
  name: string;
  url: string;
  contentType: string | null;
  concurrentRate: string | null;
  loginUrl: string | null;
  loginUi: string | null;
  loginCheckJs: string | null;
  header: string | null;
  jsLib: string | null;
  enabledCookieJar: boolean | null;
  enableDangerousApi: boolean | null;
}

export type HttpTtsConfigMutation =
  | { kind: "local" }
  | { kind: "url"; url: string }
  | { kind: "save"; id: string | null; json: string }
  | { kind: "delete"; configIds: string[] };

export type HttpTtsConfigMutationResponse =
  | { cancelled: true }
  | { items: HttpTtsConfigMetadata[]; deleted?: boolean; resource: ResourceDescriptor };

export type HttpTtsAudioResult =
  | { status: "empty" }
  | { status: "ready"; audioId: string; resource: ResourceDescriptor; contentType: string };

export interface ReadingProgress {
  chapterId: string | null;
  chapterIndex: number;
  offset: number;
  updatedAtMs: number;
}

export type SourceMutation =
  | { kind: "local" }
  | { kind: "url"; url: string }
  | { kind: "save"; id: string | null; json: string }
  | { kind: "delete"; sourceIds: string[] }
  | { kind: "login"; sourceId: string; credentials: Record<string, string> }
  | { kind: "runLoginAction"; sourceId: string; actionId: number; credentials: Record<string, string> }
  | { kind: "clearLoginState"; sourceId: string }
  | { kind: "startLoginWeb"; sourceId: string }
  | { kind: "finishLoginWeb"; sourceId: string; sessionId: string }
  | { kind: "cancelLoginWeb"; sourceId: string; sessionId: string };

export type SourceMutationResponse =
  | {
      resource: ResourceDescriptor;
      cleanupPending?: boolean;
      cleanupWarning?: string;
      rssState?: ResourceDescriptor;
    }
  | SourceLoginResult
  | SourceWebLoginResult
  | { cleared: boolean }
  | { sessionId: string }
  | { cancelled: boolean };

export interface AppSettingsResource {
  schemaVersion: number;
  sourceHttpTimeoutSeconds: number;
  reader: ReaderSettings;
  lastBackupAtMs?: number | null;
}

export interface AppBootstrap {
  shelf: ResourceDescriptor;
  settings: ResourceDescriptor;
  httpTtsConfigs: ResourceDescriptor;
  txtTocRules: ResourceDescriptor;
  replacementRules: ResourceDescriptor;
  searchHistory: ResourceDescriptor;
  sources: ResourceDescriptor;
}

export interface SearchResponse {
  resource: ResourceDescriptor;
  bookCount: number;
  errors: Array<{ sourceId?: string; message: string }>;
}

export interface SourceDebugResponse {
  ok: boolean;
  stage?: "result" | "source" | "detail" | "catalog" | "chapter";
  error?: string;
  cleared?: boolean;
  book?: {
    title: string;
    author?: string;
    intro?: string;
    kind?: string;
    wordCount?: string;
    latestChapter?: string;
    sourceId: string;
    sourceName: string;
    sourceGroup?: string;
  };
  chapters?: Array<{ index: number; title: string }>;
  chapterResource?: ResourceDescriptor | null;
}

export interface BookMutationResponse {
  book: ResourceDescriptor;
  shelf: ResourceDescriptor;
}

export type RefreshBookInfoResponse =
  | {
      commitState: "committed";
      recoveryRequired: boolean;
      warning: string | null;
      book: ResourceDescriptor;
      shelf: ResourceDescriptor;
    }
  | {
      commitState: "notCommitted" | "indeterminate";
      recoveryRequired: boolean;
      warning: string | null;
      book?: never;
      shelf?: never;
    };

export type ChangeChapterSourceResponse =
  | {
      commitState: "committed";
      recoveryRequired: boolean;
      warning: string | null;
      error?: string;
      book: ResourceDescriptor;
      chapterId: string;
    }
  | {
      commitState: "notCommitted" | "indeterminate";
      recoveryRequired: boolean;
      warning: string | null;
      error?: string;
      book?: never;
      chapterId?: never;
    };

export interface BookSourceMutationResponse extends BookMutationResponse {
  progress: ReadingProgress;
  movedProgress: boolean;
  bookmarks: {
    resource: ResourceDescriptor;
    migratedCount: number;
    orphanedCount: number;
  };
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

export interface PendingExternalFile {
  token: string;
  filename: string;
  format: "txt" | "epub" | "cbz" | "pdf";
}

export interface AppTask {
  id: string;
  kind: string;
  status: string;
  searchId?: string;
  bookId?: string;
  chapterIds?: string[];
  downloadSnapshotValid?: boolean;
  sourceIds?: string[];
  keyword?: string;
  checkOnly?: boolean;
  page: number;
  fromIndex: number;
  total: number;
  completed: number;
  createdAtMs: number;
  updatedAtMs: number;
  error?: string;
  result?: {
    /** 换源候选任务完成后发布的不可变结果版本。 */
    resource?: ResourceDescriptor;
    bookResourceId?: string;
    addedCount?: number;
    movedProgress?: boolean;
    committed?: boolean;
    commitState?: "committed" | "notCommitted" | "indeterminate";
    recoveryRequired?: boolean;
    /** Transient only: tasks.json has a terminal status, but search publication failed. */
    publicationFailed?: boolean;
    persistedTaskStatus?: string;
    warning?: string;
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
  reused?: boolean;
}

export interface ClearFinishedTasksResponse {
  removedCount: number;
  inUseCount: number;
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

export interface DiscoveryResponse {
  sourceId: string;
  categoryId?: string;
  categoryCount?: number;
  page?: number;
  hasNextPage?: boolean;
  bookCount?: number;
  errors?: Array<{ sourceId?: string; message: string }>;
  resource: ResourceDescriptor;
}

export interface BackupResponse {
  cancelled?: boolean;
  backup?: boolean;
  created?: boolean;
  warning?: string;
  settings?: ResourceDescriptor;
  restored?: boolean;
  bootstrap?: AppBootstrap;
}

export interface WebDavBackupConfig {
  url: string;
  username: string;
  hasPassword: boolean;
  autoIntervalHours?: number | null;
}

export interface WebDavBackupUploadResponse {
  uploaded: boolean;
  settings?: ResourceDescriptor;
  warning?: string;
}

export type BackupRestoreOutcome =
  | {
      restored: true;
      commitState: "committed";
      recoveryRequired: false;
      warning: string | null;
      bootstrap: AppBootstrap;
    }
  | {
      restored: false;
      commitState: "notCommitted";
      recoveryRequired: boolean;
      warning: string;
      bootstrap?: never;
    }
  | {
      restored: false;
      commitState: "committed" | "indeterminate";
      recoveryRequired: true;
      warning: string;
      bootstrap?: never;
    };

export type RestoreBackupResponse = { cancelled: true } | BackupRestoreOutcome;
export type WebDavBackupRestoreResponse = BackupRestoreOutcome;

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

export type DictionaryLanguage = "zh" | "en";

export interface DictionaryLookupResult {
  title: string;
  provider: string;
  sourceUrl: string;
  resource: ResourceDescriptor;
}

// Full book-source JSON stays separate from summary metadata and is read from
// the AppBootstrap sources resource only by the source management feature.
export type SourceDefinition = Record<string, unknown>;

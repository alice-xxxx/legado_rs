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
  contentType?: "text" | "image" | "audio" | "video";
  searchType?: SearchType;
  mediaType?: "audio" | "video";
  capabilities?: { search: boolean; detail: boolean; toc: boolean; content: boolean };
  userAgentOverride?: string | null;
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

export type SourceLoginLayoutRow = (
  { kind: "field"; name: string } |
  { kind: "action"; id: number } |
  { kind: "title"; label: string }
) & { cols: number; rows: number };

export type SourceLoginForm = {
  mode: "form";
  canLogin: boolean;
  fields: SourceLoginField[];
  actions: SourceLoginAction[];
  layout?: SourceLoginLayoutRow[];
} | {
  mode: "web";
};

export type SearchType = "text" | "audio" | "image" | "video";
export type ContentType = SearchType | "html" | "pdf";

export interface ContentPreview {
  id: string;
  title: string;
  author?: string;
  cover?: string;
  total?: number;
  latest?: string;
  read?: number;
  readAt?: number;
  updatedAt?: number;
}
export interface ContentGroup { id: string[]; name: string; }
export type ContentLibrary = Partial<Record<ContentType, ContentPreview[]>> & { groups?: ContentGroup[] };

export interface ContentAlternative {
  resultId: string;
  bookUrl: string;
  sourceId: string;
  sourceName: string;
  title: string;
  author?: string;
  cover?: string;
  latest?: string;
  intro?: string;
  responseTimeMs?: number;
}
export interface ImagePage { id: string; src: string; url?: string; }
export interface ChapterImagesResource { kind: "images"; pages: ImagePage[]; }
export type ChapterContentFormat = "text" | "richText" | "image" | "audio" | "video" | "pdf";

export interface ChapterResourceDescriptor extends ResourceDescriptor {
  /** Semantic chapter kind supplied by Rust; do not infer it from URL or MIME type. */
  contentFormat: ChapterContentFormat;
}

export interface ChapterResource {
  contentId?: string;
  volume?: boolean;
  id: string;
  title: string;
  index: number;
  /** The sole location and semantic format for a prepared chapter. */
  resource?: ChapterResourceDescriptor;
  pages?: ImagePage[];
  media?: ChapterMediaResource;
  pdfPage?: ChapterPdfPageResource;
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
  bookUrl?: string;
  schemaVersion: number;
  id: string;
  title: string;
  author?: string;
  intro?: string;
  kind?: string;
  contentType?: ContentType;
  /** Filename of the retained local original, such as original.epub. */
  original?: string;
  updatedAtMs?: number;
  /** 最近一次成功获取详情和目录的时间。 */
  informationFetchedAtMs?: number;
  cover?: string;
  alternatives?: ContentAlternative[];
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
    pageId?: string;
    anchor?: string;
    quality?: string;
    chapterIndex?: number;
    offset?: number;
    updatedAtMs?: number;
  };
  chapters: ChapterResource[];
}

/** Shared list metadata. Complete catalogs belong only to openedBook. */
export type BookPreview = Pick<BookResource,
  "id" | "title" | "author" | "coverSrc" | "kind" | "contentType" | "mediaType" | "chapterCount" |
  "latestChapter" | "progress" | "canChangeSource" | "sourceId" | "sourceName"
>;

/** Partial save payload: only the book ID and fields editable in the details UI. */
export interface BookMetadataUpdate {
  id: string;
  displayOverrides?: Partial<Pick<BookDisplayMetadataValues, "title" | "author" | "intro" | "coverSrc">>;
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
  | { cancelled: false; image: ResourceDescriptor };

export interface ShelfDocument {
  schemaVersion: number;
  sort?: ShelfSortKey;
  sortOrder?: ShelfSortOrder;
  groups?: string[];
  books: Array<{ id: string; groups?: string[] }>;
}

/** Browser view derived from shelf positions and the cached book documents. */
export interface ShelfResource {
  schemaVersion: number;
  sort?: ShelfSortKey;
  sortOrder?: ShelfSortOrder;
  groups?: string[];
  books: Array<Pick<BookResource, "id" | "title" | "author" | "coverSrc" | "chapterCount" | "latestChapter" | "progress" | "kind" | "contentType" | "mediaType"> & { groups?: string[]; updatedAt?: number }>;
}

/** 请求成功即缓存字段已提交；warning 表示文件清理尚待恢复。 */
export interface ClearBookChapterCacheResponse {
  clearedCount: number;
  warning: string | null;
}

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
  pageId?: string;
  anchor?: string;
  id: string;
  bookId: string;
  chapterId: string;
  chapterIndex: number;
  chapterTitle?: string;
  offset: number;
  note: string;
  orphaned?: boolean;
  createdAtMs: number;
  updatedAtMs: number;
}

export type BookmarkRecord = Omit<Bookmark, "bookId">;

export interface ReadingHistoryResource {
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

/** Rust 处理后的单个书源候选；resultId 用于详情、入架和换源操作。 */
export interface SearchResultSource {
  /** 本次书源搜索耗时，包含网站请求和规则处理，不含本地排队。 */
  responseTimeMs?: number;
  /** 原始书籍上下文供统一写库保存，详情规则可继续使用其中的自定义字段。 */
  engine?: Record<string, unknown>;
  bookUrl: string;
  alternativeIndex?: number;
  resultId: string;
  sourceId: string;
  sourceName: string;
  contentType?: SearchType;
  mediaType?: "audio" | "video";
  kind?: string;
  articleId?: string;
  contentSrc?: string;
  isRead?: boolean;
  isFavorite?: boolean;
  title: string;
  author?: string;
  coverSrc?: string;
  intro?: string;
  latestChapter?: string;
  [key: string]: unknown;
}

/** Vue 合并后的展示书目，sources 保存可换来源。 */
export interface SearchBookResult extends SearchResultSource {
  sources?: SearchResultSource[];
}

export interface SearchResource {
  searchId?: string;
  keyword: string;
  page: number;
  results: SearchResultSource[];
  errors: Array<{ sourceId?: string; message: string }>;
  complete?: boolean;
}

export type SearchPrecision = "exact" | "precise" | "none";

/** 每个源只发送一次过滤结果；search 返回即结束。首次通知仅提供 searchId。 */
export interface SearchUpdate {
  searchId: string;
  sourceId?: string;
  results: SearchResultSource[];
  hasMore?: boolean;
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
  entries: SearchHistoryEntry[];
}

export interface HistoryResource {
  schemaVersion: number;
  reading: ReadingHistoryResource;
  search: SearchHistoryResource;
  /** Book ID -> full bookmark records; the key supplies each record's bookId. */
  bookmarks: Record<string, BookmarkRecord[]>;
}

export type AppTheme = "light" | "dark" | "system";
export type ReadingTheme = "white" | "paper" | "sepia" | "dark";

export interface ReaderSettings {
  fontSizePx: number;
  lineHeight: number;
  fontFamily: string;
  /** Runtime URL materialized by the resource server from a stable resource ref. */
  backgroundImageSrc?: string;
  /** Visible opacity of the reader background image, from 0 to 100 percent. */
  backgroundImageOpacity: number;
  textAlign: string;
  themes: Record<Exclude<AppTheme, "system">, ReadingTheme>;
  preloadCount: number;
  readingMode: "none" | "cover" | "slide" | "slide-vertical" | "fade" | "flip" | "simulation" | "vertical";
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

/** Only the fields edited by the local form; saving updates the shared rule. */
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

export interface ReadingProgress {
  pageId?: string | null;
  anchor?: string | null;
  quality?: string | null;
  chapterId: string | null;
  chapterIndex: number;
  offset: number;
  updatedAtMs: number;
}

export interface AppSettingsResource {
  webdavBackup?: WebDavBackupConfig;
  schemaVersion: number;
  theme: AppTheme;
  sourceHttpTimeoutSeconds: number;
  reader: ReaderSettings;
  home: HomeConfigDocument;
  lastBackupAtMs?: number | null;
}

export interface MutationAck { updated: true; }
export interface BookIdentity { bookId: string; }
export interface AppBootstrap {
  assetBaseUrl: string;
  settings: AppSettingsResource;
}

export interface LocalBookImportOptions {
  charset?: string | null;
  tocRegex?: string | null;
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
  search?: SearchResource;
  sourceId: string;
  categoryId?: string;
  categoryCount?: number;
  page?: number;
  hasNextPage?: boolean;
  bookCount?: number;
  errors?: Array<{ sourceId?: string; message: string }>;
}

export interface BackupResponse {
  cancelled?: boolean;
  backup?: boolean;
  created?: boolean;
  warning?: string;
  settings?: AppSettingsResource;
  restored?: boolean;
  bootstrap?: AppBootstrap;
}

export interface WebDavBackupConfig {
  url: string;
  username: string;
  password: string | null;
  autoIntervalHours?: number | null;
  lastWebdavBackupAtMs?: number | null;
  autoBackupScheduledAtMs?: number | null;
  lastAutoAttemptAtMs?: number | null;
}

export interface WebDavBackupUploadResponse {
  uploaded: boolean;
  settings?: AppSettingsResource;
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
  categoryId: string;
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
  tabs: HomeTab[];
}

export type DictionaryLanguage = "zh" | "en";

export interface DictionaryLookupResult {
  title: string;
  provider: string;
  sourceUrl: string;
  resource: ResourceDescriptor;
}

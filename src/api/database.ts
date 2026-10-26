import Database from "@tauri-apps/plugin-sql";
import type { AppBootstrap, BookPreview, BookResource, ChapterResource, ContentAlternative, ContentLibrary, ContentType, DiscoveryCategoriesResource, HistoryResource, ReadingHistoryResource, SourceMetadata } from "./types";

// Rust 在 WebView 启动前安装只读连接，前端只调用 select。
const database = Database.get("sqlite:library-reader");
let assetBase = "";
export function initializeDatabase(bootstrap: AppBootstrap): void { assetBase = bootstrap.assetBaseUrl; }
export function assetUrl(kind: ContentType, id: string, name: string): string {
  if (/^https?:\/\//.test(name)) return name;
  const path = name.startsWith("resource://") ? name.slice(11) : `cache/${kind}/${encodeURIComponent(id)}/${encodeURIComponent(name)}`;
  return new URL(path, assetBase).href;
}

/** 数据库存稳定资源地址，不保存带端口和令牌的运行时访问地址。 */
export function storedAssetUrl(url: string): string {
  return assetBase && url.startsWith(assetBase) ? `resource://${url.slice(assetBase.length)}` : url;
}

type Row = Record<string, any>;
type Table = { table: string; fields: string; where?: string };
function fields(mapping: string): string[] { return mapping.split(" ").filter(Boolean); }
// 一个普通 SELECT 读取关联表，保证共享同一份 WAL 快照。
async function tables(specs: Table[], args: unknown[] = []): Promise<Row[][]> {
  const mappings = specs.map(spec => fields(spec.fields).map(field => { const [column, name = column] = field.split(":"); return { column, name }; }));
  const names = [...new Set(mappings.flat().map(field => field.name))];
  const sql = specs.map((spec, part) => {
    const columns = names.map(name => `${mappings[part]!.find(field => field.name === name)?.column ?? "NULL"} AS "${name}"`).join(",");
    return `SELECT * FROM (SELECT ${part} AS __part,${columns} FROM ${spec.table} ${spec.where ?? ""})`;
  }).join(" UNION ALL ");
  const groups: Row[][] = specs.map(() => []);
  for (const row of await database.select<Row[]>(sql, args)) {
    const part = row.__part as number;
    groups[part]!.push(Object.fromEntries(mappings[part]!.map(({ name }) => [name, row[name]])));
  }
  return groups;
}
function optional(row: Row): Row { return Object.fromEntries(Object.entries(row).filter(([, value]) => value !== null)); }
function booleans(row: Row, ...names: string[]): Row { for (const name of names) if (name in row) row[name] = !!row[name]; return row; }

export async function readSources(): Promise<SourceMetadata[]> {
  const rows = await database.select<Row[]>(`SELECT id,name,source_group AS "group",enabled,is_rss AS isRss,content_type AS contentType,search_type AS searchType,media_type AS mediaType,can_search,can_detail,can_toc,can_content,user_agent_override AS userAgentOverride FROM source_records WHERE removed=0 ORDER BY position`);
  return rows.map(row => ({ ...optional(booleans(row, "enabled", "isRss")), capabilities: { search: !!row.can_search, detail: !!row.can_detail, toc: !!row.can_toc, content: !!row.can_content } })) as SourceMetadata[];
}
const sourceOptions = "book_source_type book_url_pattern enabled_explore enabled_review js_lib enabled_cookie_jar enable_dangerous_api concurrent_rate login_url login_check_js cover_decode_js book_source_comment variable_comment last_update_time respond_time weight explore_url explore_screen explore_style search_url";
const ttsOptions = "id name url content_type concurrent_rate login_url login_check_js js_lib enabled_cookie_jar enable_dangerous_api last_update_time";
function profileTables(prefix: "source" | "http_tts"): Table[] {
  const options = prefix === "source" ? sourceOptions : ttsOptions;
  const mapping = (options + " header_script login_ui_script").split(" ").map(column => `${column}:${column.replace(/_([a-z])/g, (_, char: string) => char.toUpperCase())}`).join(" ");
  return [
    { table: `${prefix}_options`, fields: `owner_id:ownerId ${mapping}` },
    { table: `${prefix}_headers`, fields: `owner_id:ownerId name value`, where: "ORDER BY position" },
    { table: `${prefix}_login_items`, fields: `owner_id:ownerId position name type action cols rows width`, where: "ORDER BY position" },
    { table: `${prefix}_login_choices`, fields: `owner_id:ownerId item_position:itemPosition value`, where: "ORDER BY position" },
  ];
}
function profile(id: string, data: Row[][]): Row {
  const [options, headers, items, choices] = data.map(rows => rows.filter(row => row.ownerId === id));
  const value = booleans(optional({ ...options![0] }), "enabledExplore", "enabledReview", "enabledCookieJar", "enableDangerousApi");
  const login = items!.map(row => {
    const item = optional({ ...row });
    delete item.ownerId; delete item.position;
    const entries = choices!.filter(choice => choice.itemPosition === row.position).map(choice => choice.value);
    if (entries.length) item.options = entries;
    return item;
  });
  const header = Object.fromEntries(headers!.map(row => [row.name, row.value]));
  if (value.headerScript !== undefined) value.header = value.headerScript;
  else if (headers!.length) value.header = JSON.stringify(header);
  if (value.loginUiScript !== undefined) value.loginUi = value.loginUiScript;
  else if (login.length) value.loginUi = JSON.stringify(login);
  delete value.ownerId; delete value.headerScript; delete value.loginUiScript;
  return value;
}
export async function readSourceDefinition(id: string): Promise<string> {
  const [sources, rules, links, ...data] = await tables([
    { table: "sources", fields: "id url:bookSourceUrl name:bookSourceName source_group:bookSourceGroup enabled user_agent_override:legadoRsUserAgentOverride", where: "WHERE id=$1" },
    { table: "source_rules", fields: "stage target expression", where: "WHERE source_id=$1" },
    { table: "source_explore_links", fields: "title url kind", where: "WHERE source_id=$1 ORDER BY position" },
    ...profileTables("source").map(spec => ({ ...spec, where: `WHERE owner_id=$1 ${spec.where ?? ""}` })),
  ], [id]);
  if (!sources![0]) throw new Error("书源不存在");
  const value = { ...profile(id, data), ...booleans(optional(sources![0]), "enabled") };
  delete value.id;
  for (const rule of rules!) (value[rule.stage] ??= {})[rule.target] = rule.expression;
  if (links!.length) value.exploreUrl = JSON.stringify(links!.map(optional));
  return JSON.stringify(value);
}
export async function readConfiguration<T>(kind: "replacementRules" | "txtTocRules" | "httpTtsConfigs", id?: string): Promise<T> {
  const where = id === undefined ? "ORDER BY position" : "WHERE id=$1";
  const args = id === undefined ? [] : [id];
  if (kind === "replacementRules") {
    const [rows] = await tables([{ table: "replacement_rules", fields: "id name pattern replacement enabled is_regex:isRegex scope", where }], args);
    return { schemaVersion: 1, rules: rows!.map(row => booleans(row, "enabled", "isRegex")) } as T;
  }
  if (kind === "txtTocRules") {
    const [rows] = await tables([{ table: "txt_toc_rules", fields: "id name rule example serial_number:serialNumber enable", where }], args);
    return { schemaVersion: 1, rules: rows!.map(row => booleans(optional(row), "enable")) } as T;
  }
  const [rows, ...data] = await tables([
    { table: "http_tts_configs", fields: "id name enabled", where },
    ...profileTables("http_tts").map(spec => id === undefined ? spec : { ...spec, where: `WHERE owner_id=$1 ${spec.where ?? ""}` }),
  ], args);
  return { schemaVersion: 1, configs: rows!.map(row => ({ ...booleans(row, "enabled"), config: profile(row.id, data) })) } as T;
}

const previewFields = `id title author cover kind source_id:sourceId source_name:sourceName chapter_count:chapterCount latest_title:latestChapter can_change_source:canChangeSource chapter_id:chapterId chapter_index:chapterIndex page_id:pageId anchor quality offset progress_updated_at_ms:updatedAtMs`;
function preview(row: Row): BookPreview {
  return { ...optional(row), contentType: row.kind, mediaType: row.kind === "audio" || row.kind === "video" ? row.kind : undefined,
    canChangeSource: !!row.canChangeSource, coverSrc: row.cover ? assetUrl(row.kind, row.id, row.cover) : undefined,
    progress: { chapterId: row.chapterId, chapterIndex: row.chapterIndex ?? 0, pageId: row.pageId, anchor: row.anchor, quality: row.quality, offset: row.offset, updatedAtMs: row.updatedAtMs } } as BookPreview;
}
export async function readBookPreview(id: string): Promise<BookPreview> {
  const [rows] = await tables([{ table: "book_previews", fields: previewFields, where: "WHERE id=$1" }], [id]);
  if (!rows![0]) throw new Error("书籍不存在");
  return preview(rows![0]);
}
export async function readLibrary(): Promise<ContentLibrary> {
  const [books, groups, members, lastChapters] = await tables([
    { table: "book_previews b JOIN library_books l ON l.book_id=b.id LEFT JOIN chapter_positions c ON c.book_id=b.id AND c.id=b.chapter_id", fields: "b.id b.kind b.title b.author b.cover b.readable_count:total b.latest_title:latest b.updated_at_ms:updatedAt b.progress_updated_at_ms:readAt coalesce(c.read_position,b.chapter_index+1):read", where: "ORDER BY l.position" },
    { table: "library_groups", fields: "name", where: "ORDER BY position" },
    { table: "library_group_books g JOIN library_books b ON b.book_id=g.book_id", fields: "g.group_name:name g.book_id:id", where: "ORDER BY b.position" },
    { table: "library_books l JOIN chapters c ON c.book_id=l.book_id AND c.position=(SELECT max(position) FROM chapters WHERE book_id=l.book_id AND volume=0)", fields: "c.book_id:id c.title:title" },
  ]);
  const library: ContentLibrary = {};
  const lastTitles = new Map(lastChapters!.map(row => [row.id, row.title]));
  for (const row of books!) {
    const kind = row["b.kind"] as ContentType, id = row["b.id"] as string;
    const latest = row.latest?.trim();
    (library[kind] ??= []).push({ id, title: row["b.title"], author: row["b.author"] ?? undefined, cover: row["b.cover"] ?? undefined,
      total: row.total, latest: !latest || /^共\s*\d+\s*[章节话集期页]?$/.test(latest) ? lastTitles.get(id) : latest,
      updatedAt: row.updatedAt ? Math.floor(row.updatedAt / 1000) : undefined,
      readAt: row.readAt ? Math.floor(row.readAt / 1000) : undefined, read: row.readAt ? row.read ?? 0 : undefined });
  }
  const groupMembers = new Map<string, string[]>();
  for (const row of members!) { const ids = groupMembers.get(row.name) ?? []; ids.push(row.id); groupMembers.set(row.name, ids); }
  library.groups = groups!.map(row => ({ name: row.name, id: groupMembers.get(row.name) ?? [] }));
  return library;
}

const chapterFields = "id content_id:contentId position:index title volume url duration_ms:durationMs protocol";
const pageFields = "chapter_id:chapterId id url width height";
const streamFields = "chapter_id:chapterId label url protocol";

function chapterDescriptor(type: ContentType, id: string, url: string) {
  return { resourceId: url.startsWith("resource://") || /^https?:/.test(url) ? url : `resource://cache/${type}/${id}/${url}`, src: assetUrl(type, id, url),
    contentType: type === "html" ? "text/html" : type === "pdf" ? "application/pdf" : "text/plain", format: type === "html" ? "html" : type, contentFormat: type === "html" ? "richText" : type };
}

function chapterResources(book: Pick<BookResource, "id" | "contentType" | "progress">, chapters: Row[], pages: Row[], streams: Row[]): ChapterResource[] {
  const { id, progress } = book, type = book.contentType ?? "text";
  const byChapter = (values: Row[]) => { const map = new Map<string, Row[]>(); for (const value of values) { const entries = map.get(value.chapterId) ?? []; entries.push(value); map.set(value.chapterId, entries); } return map; };
  const pageMap = byChapter(pages), streamMap = byChapter(streams);
  return chapters.map(chapter => {
    const next: Row = { ...optional(chapter), volume: !!chapter.volume };
    if (chapter.url) next.resource = chapterDescriptor(type, id, chapter.url);
    if (pageMap.has(chapter.id)) next.pages = pageMap.get(chapter.id)!.map(page => ({ ...optional(page), url: chapterDescriptor(type, id, page.url).resourceId, src: assetUrl(type, id, page.url) }));
    if (type === "audio" || type === "video") {
      const sources = streamMap.get(chapter.id) ?? [];
      const selected = progress?.chapterId === chapter.id ? sources.find(source => source.label === progress?.quality && source.url) : undefined;
      const selectedUrl = selected?.url ?? chapter.url;
      if (selectedUrl) next.media = { schemaVersion: 1, kind: "media", mediaType: type, src: assetUrl(type, id, selectedUrl), format: selected?.protocol ?? chapter.protocol ?? "direct",
        sources: sources.map(source => ({ label: source.label, src: source.url ? assetUrl(type, id, source.url) : undefined, format: source.protocol ?? "direct", selected: source.url === selectedUrl })) };
    }
    return next as ChapterResource;
  });
}

/** 缓存变化只读这一章，图片和媒体地址与章节共享同一快照。 */
export async function readChapter(book: Pick<BookResource, "id" | "contentType" | "progress">, chapterId: string): Promise<{ contentType: ContentType; chapter: ChapterResource } | null> {
  const [metadata, chapters, pages, streams] = await tables([
    { table: "books", fields: "content_type:kind", where: "WHERE id=$1" },
    { table: "chapter_records", fields: chapterFields, where: "WHERE book_id=$1 AND id=$2" },
    { table: "chapter_pages", fields: pageFields, where: "WHERE book_id=$1 AND chapter_id=$2 ORDER BY position" },
    { table: "chapter_streams", fields: streamFields, where: "WHERE book_id=$1 AND chapter_id=$2 ORDER BY position" },
  ], [book.id, chapterId]);
  const contentType = metadata![0]?.kind as ContentType | undefined;
  if (!contentType) return null;
  const chapter = chapterResources({ ...book, contentType }, chapters!, pages!, streams!)[0];
  return chapter ? { contentType, chapter } : null;
}

export async function readBookAlternatives(id: string): Promise<ContentAlternative[]> {
  const [rows] = await tables([{ table: "alternative_sources", fields: "alternative_id:resultId source_id:sourceId source_name:sourceName engine_url:bookUrl title author cover latest", where: "WHERE book_id=$1 ORDER BY position" }], [id]);
  return rows!.map(optional) as ContentAlternative[];
}

export async function readBook(id: string): Promise<BookResource> {
  const [metadata, chapters, pages, streams, tags, alternatives, progressRows] = await tables([
    { table: "book_records", fields: "id content_type:kind title author intro cover source_id:sourceId source_name:sourceName source_group:sourceGroup book_url:bookUrl word_count:wordCount original page_count:pageCount url updated_at_ms:updatedAtMs information_fetched_at_ms:informationFetchedAtMs chapter_count:chapterCount latest_title:latestChapter title_override:titleOverride author_override:authorOverride intro_override:introOverride cover_override:coverOverride (book_url!=char(0)):hasEngine", where: "WHERE id=$1" },
    { table: "chapter_records", fields: chapterFields, where: "WHERE book_id=$1 ORDER BY position" },
    { table: "chapter_pages", fields: pageFields, where: "WHERE book_id=$1 ORDER BY chapter_id,position" },
    { table: "chapter_streams", fields: streamFields, where: "WHERE book_id=$1 ORDER BY chapter_id,position" },
    { table: "book_tags", fields: "tag", where: "WHERE book_id=$1 ORDER BY position" },
    { table: "alternative_sources", fields: "alternative_id:resultId source_id:sourceId source_name:sourceName engine_url:bookUrl title author cover latest", where: "WHERE book_id=$1 ORDER BY position" },
    { table: "book_previews", fields: previewFields, where: "WHERE id=$1" },
  ], [id]);
  // 预览和目录共用一条查询，避免目录替换时读到不同快照。
  const row = metadata![0];
  if (!row) throw new Error("书籍不存在");
  const type = row.kind as ContentType;
  const snapshot = progressRows![0];
  const progress = snapshot ? preview(snapshot).progress! : { chapterIndex: 0, offset: 0, updatedAtMs: 0 };
  const catalog = chapterResources({ id, contentType: type, progress }, chapters!, pages!, streams!);
  if (type === "pdf" && row.url) for (let index = 0; index < Math.max(row.pageCount ?? 0, 1); index++) catalog.push({ id: `page-${index + 1}`, title: String(index + 1), index,
    resource: chapterDescriptor(type, id, row.url) as ChapterResource["resource"], pdfPage: { schemaVersion: 1, kind: "pdfPage", src: assetUrl(type, id, row.url), pageIndex: index, defaultZoom: "page-fit" } });
  const base = { title: row.title, author: row.author ?? "", intro: row.intro ?? "", coverSrc: row.cover ? assetUrl(type, id, row.cover) : undefined };
  const overrides = optional({ title: row.titleOverride, author: row.authorOverride, intro: row.introOverride, coverSrc: row.coverOverride });
  if (overrides.coverSrc) overrides.coverSrc = assetUrl(type, id, overrides.coverSrc);
  return { ...optional(row), ...base, ...overrides, schemaVersion: 1, id, contentType: type, mediaType: type === "audio" || type === "video" ? type : undefined,
    kind: tags!.map(row => row.tag).join(" "), canChangeSource: !!row.sourceId && !!row.hasEngine, displayBase: base, displayOverrides: overrides,
    original: row.original ?? undefined, chapterCount: type === "pdf" ? catalog.length : row.chapterCount, progress, chapters: catalog, alternatives: alternatives!.map(optional) } as BookResource;
}

export async function readCategories(sourceId: string): Promise<DiscoveryCategoriesResource> {
  const [rows] = await tables([{ table: "discovery_categories", fields: "category_id:categoryId title kind style_cols:cols style_rows:rows style_width:layoutFlexBasisPercent", where: "WHERE source_id=$1 ORDER BY position" }], [sourceId]);
  return { schemaVersion: 1, sourceId, categories: rows!.map(row => ({ ...optional(row), style: optional({ cols: row.cols, rows: row.rows, layoutFlexBasisPercent: row.layoutFlexBasisPercent }) })) } as DiscoveryCategoriesResource;
}

export async function readSearchHistory(): Promise<HistoryResource["search"]> {
  const [entries]=await tables([
    {table:"search_history",fields:"query normalized_query:normalizedQuery usage first_use_time_ms:firstUseTimeMs last_use_time_ms:lastUseTimeMs",where:"ORDER BY last_use_time_ms DESC,usage DESC,normalized_query"},
  ]);
  return {entries} as HistoryResource["search"];
}

export async function readReadingHistory():Promise<ReadingHistoryResource>{
  const [sessions,books,days,dayBooks,totals]=await tables([
    {table:"reading_sessions",fields:"book_id:bookId session_id:sessionId duration_ms:durationMs ended_at_ms:endedAtMs utc_offset_minutes:utcOffsetMinutes",where:"ORDER BY ended_at_ms,session_id"},
    {table:"reading_books",fields:"book_id:bookId session_count:sessionCount total_duration_ms:totalDurationMs last_read_at_ms:lastReadAtMs",where:"ORDER BY last_read_at_ms DESC,book_id"},
    {table:"reading_days",fields:"day session_count:sessionCount total_duration_ms:totalDurationMs",where:"ORDER BY day DESC"},
    {table:"reading_day_books",fields:"day book_id:bookId session_count:sessionCount total_duration_ms:totalDurationMs",where:"ORDER BY day,book_id"},
    {table:"reading_totals",fields:"duration_ms:totalDurationMs sessions:totalSessions",where:"WHERE id=1"},
  ]);
  const grouped=new Map<string,Row[]>();
  for(const row of dayBooks!){const values=grouped.get(row.day) ?? [];values.push(row);grouped.set(row.day,values);}
  return {sessions,books,days:days!.map(day=>({...day,books:grouped.get(day.day) ?? []})),...totals![0]} as ReadingHistoryResource;
}

export async function readHistory(): Promise<HistoryResource> {
  const [sessions, books, days, dayBooks, entries, bookmarks, totals] = await tables([
    { table: "reading_sessions", fields: "book_id:bookId session_id:sessionId duration_ms:durationMs ended_at_ms:endedAtMs utc_offset_minutes:utcOffsetMinutes", where: "ORDER BY ended_at_ms,session_id" },
    { table: "reading_books", fields: "book_id:bookId session_count:sessionCount total_duration_ms:totalDurationMs last_read_at_ms:lastReadAtMs", where: "ORDER BY last_read_at_ms DESC,book_id" },
    { table: "reading_days", fields: "day session_count:sessionCount total_duration_ms:totalDurationMs", where: "ORDER BY day DESC" },
    { table: "reading_day_books", fields: "day book_id:bookId session_count:sessionCount total_duration_ms:totalDurationMs", where: "ORDER BY day,book_id" },
    { table: "search_history", fields: "query normalized_query:normalizedQuery usage first_use_time_ms:firstUseTimeMs last_use_time_ms:lastUseTimeMs", where: "ORDER BY last_use_time_ms DESC,usage DESC,normalized_query" },
    { table: "bookmarks", fields: "id book_id:bookId chapter_id:chapterId chapter_index:chapterIndex chapter_title:chapterTitle offset page_id:pageId anchor note orphaned created_at_ms:createdAtMs updated_at_ms:updatedAtMs", where: "ORDER BY book_id,position" },
    { table: "reading_totals", fields: "duration_ms:totalDurationMs sessions:totalSessions", where: "WHERE id=1" },
  ]);
  const grouped: Record<string, Row[]> = {};
  for (const bookmark of bookmarks!) (grouped[bookmark.bookId] ??= []).push(booleans(optional(bookmark), "orphaned"));
  const groupedDays = new Map<string, Row[]>();
  for (const row of dayBooks!) { const values = groupedDays.get(row.day) ?? []; values.push(row); groupedDays.set(row.day, values); }
  return { schemaVersion: 2, reading: { sessions, books, days: days!.map(day => ({ ...day, books: groupedDays.get(day.day) ?? [] })), ...totals![0] }, search: { entries }, bookmarks: grouped } as HistoryResource;
}

/** 成功获取详情后才算缓存；仅保存搜索候选不会记录详情获取时间。 */
export async function readCachedMoreInformation(sourceId: string, bookUrl: string): Promise<BookResource | null> {
  const rows = await database.select<Array<{ id: string }>>(`SELECT b.id FROM books b JOIN sources s ON s.id=b.source_id AND (s.content_type=b.content_type OR (s.content_type='text' AND b.content_type='html')) WHERE b.source_id=$1 AND b.book_url=$2 AND b.information_fetched_at_ms IS NOT NULL ORDER BY b.id LIMIT 1`, [sourceId, bookUrl]);
  return rows[0] ? readBook(rows[0].id) : null;
}

/** 与写入接口使用相同的来源和 URL，直接读详情存储。 */
export async function readMoreInformation(sourceId: string, bookUrl: string): Promise<BookResource> {
  const book = await readCachedMoreInformation(sourceId, bookUrl);
  if (!book) throw new Error("详情尚未保存");
  return book;
}

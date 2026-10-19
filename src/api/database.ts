import Database from "@tauri-apps/plugin-sql";
import type { AppBootstrap, AppTask, BookPreview, BookResource, ChapterResource, ContentLibrary, ContentType, DiscoveryCategoriesResource, HistoryResource, ReadingHistoryResource, SourceMetadata } from "./types";

// Rust installs this connection before the WebView starts. No load/execute API is used.
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
function expressions(mapping: string): string {
  return fields(mapping).map(field => { const [column, name = column] = field.split(":"); return `'${name}',${column}`; }).join(",");
}
// One SELECT gives all related tables the same WAL snapshot, including a catalog's children.
async function tables(specs: Table[], args: unknown[] = []): Promise<Row[][]> {
  const sql = specs.map((spec, index) => `SELECT ${index} AS part, coalesce(json_group_array(json(entry)), '[]') AS rows FROM (SELECT json_object(${expressions(spec.fields)}) AS entry FROM ${spec.table} ${spec.where ?? ""})`).join(" UNION ALL ");
  const result = await database.select<Array<{ part: number; rows: string }>>(sql, args);
  return result.sort((a,b) => a.part-b.part).map(({ rows }) => JSON.parse(rows) as Row[]);
}
function optional(row: Row): Row { return Object.fromEntries(Object.entries(row).filter(([, value]) => value !== null)); }
function booleans(row: Row, ...names: string[]): Row { for (const name of names) if (name in row) row[name] = !!row[name]; return row; }

export async function readSources(): Promise<SourceMetadata[]> {
  const rows = await database.select<Row[]>(`SELECT id,name,source_group AS "group",enabled,is_rss AS isRss,content_type AS contentType,search_type AS searchType,media_type AS mediaType,can_search,can_detail,can_toc,can_content,user_agent_override AS userAgentOverride FROM sources ORDER BY position`);
  return rows.map(row => ({ ...optional(booleans(row, "enabled", "isRss")), capabilities: { search: !!row.can_search, detail: !!row.can_detail, toc: !!row.can_toc, content: !!row.can_content } })) as SourceMetadata[];
}
export async function readSourceDefinition(id: string): Promise<string> {
  const rows = await database.select<Array<{ definition: string }>>("SELECT definition_json AS definition FROM sources WHERE id=$1", [id]);
  if (!rows[0]) throw new Error("书源不存在");
  return rows[0].definition;
}
export async function readConfiguration<T>(kind: "replacementRules" | "txtTocRules" | "httpTtsConfigs", id?: string): Promise<T> {
  const where = id === undefined ? "ORDER BY position" : "WHERE id=$1";
  const args = id === undefined ? [] : [id];
  if (kind === "replacementRules") {
    const [rows] = await tables([{ table: "replacement_rules", fields: "id name pattern replacement enabled is_regex:isRegex scope", where }], args);
    return { schemaVersion: 1, rules: rows!.map(row => booleans(row, "enabled", "isRegex")) } as T;
  }
  if (kind === "txtTocRules") {
    const [rows] = await tables([{ table: "txt_toc_rules", fields: "id name rule example serial_number:serialNumber enabled:enable", where }], args);
    return { schemaVersion: 1, rules: rows!.map(row => booleans(optional(row), "enable")) } as T;
  }
  const [rows] = await tables([{ table: "http_tts_configs", fields: "id name enabled definition_json:config", where }], args);
  return { schemaVersion: 1, configs: rows!.map(row => ({ ...booleans(row, "enabled"), config: JSON.parse(row.config) })) } as T;
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
  const [books, groups, members] = await tables([
    { table: "book_previews b JOIN library_books l ON l.book_id=b.id LEFT JOIN chapters c ON c.book_id=b.id AND c.id=b.chapter_id", fields: "b.id b.kind b.title b.author b.cover b.readable_count:total b.latest_title:latest b.updated_at_ms:updatedAt b.progress_updated_at_ms:readAt coalesce(c.read_position,b.chapter_index+1):read", where: "ORDER BY l.position" },
    { table: "library_groups", fields: "name", where: "ORDER BY position" },
    { table: "library_group_books", fields: "group_name:name book_id:id", where: "ORDER BY position" },
  ]);
  const library: ContentLibrary = {};
  for (const row of books!) {
    const kind = row["b.kind"] as ContentType, id = row["b.id"] as string;
    (library[kind] ??= []).push({ id, title: row["b.title"], author: row["b.author"] ?? undefined, cover: row["b.cover"] ?? undefined,
      total: row.total, latest: row.latest ?? undefined, updatedAt: row.updatedAt ? Math.floor(row.updatedAt / 1000) : undefined,
      readAt: row.readAt ? Math.floor(row.readAt / 1000) : undefined, read: row.readAt ? row.read ?? 0 : undefined });
  }
  const groupMembers = new Map<string, string[]>();
  for (const row of members!) { const ids = groupMembers.get(row.name) ?? []; ids.push(row.id); groupMembers.set(row.name, ids); }
  library.groups = groups!.map(row => ({ name: row.name, id: groupMembers.get(row.name) ?? [] }));
  return library;
}

export async function readBook(id: string): Promise<BookResource> {
  const [metadata, chapters, pages, streams, tags, alternatives, progressRows] = await tables([
    { table: "books", fields: "id kind title author intro cover source_id:sourceId source_name:sourceName source_group:sourceGroup coalesce(json_extract(engine_json,'$.bookUrl'),json_extract(engine_json,'$.url')):bookUrl word_count:wordCount original_json:original page_count:pageCount document_url:url updated_at_ms:updatedAtMs chapter_count:chapterCount latest_title:latestChapter title_override:titleOverride author_override:authorOverride intro_override:introOverride cover_override:coverOverride (engine_json!=char(0)):hasEngine", where: "WHERE id=$1" },
    { table: "chapters", fields: "id position:index title is_volume:volume resource_url:url duration_ms:durationMs protocol", where: "WHERE book_id=$1 ORDER BY position" },
    { table: "chapter_pages", fields: "chapter_id:chapterId id url width height", where: "WHERE book_id=$1 ORDER BY chapter_id,position" },
    { table: "chapter_streams", fields: "chapter_id:chapterId label url protocol", where: "WHERE book_id=$1 ORDER BY chapter_id,position" },
    { table: "book_tags", fields: "tag", where: "WHERE book_id=$1 ORDER BY position" },
    { table: "alternative_sources", fields: "source_id:sourceId source_name:sourceName engine_url:bookUrl title author cover latest", where: "WHERE book_id=$1 ORDER BY position" },
    { table: "book_previews", fields: previewFields, where: "WHERE id=$1" },
  ], [id]);
  // The preview is selected in the same statement; no separate metadata request races a catalog replacement.
  const row = metadata![0];
  if (!row) throw new Error("书籍不存在");
  const type = row.kind as ContentType;
  const byChapter = (values: Row[]) => { const map = new Map<string, Row[]>(); for (const value of values) { const entries = map.get(value.chapterId) ?? []; entries.push(value); map.set(value.chapterId, entries); } return map; };
  const pageMap = byChapter(pages!), streamMap = byChapter(streams!);
  const snapshot = progressRows![0];
  const progress = snapshot ? preview(snapshot).progress! : { chapterIndex: 0, offset: 0, updatedAtMs: 0 };
  function descriptor(url: string) {
    const src = assetUrl(type, id, url);
    return { resourceId: url.startsWith("resource://") || /^https?:/.test(url) ? url : `resource://cache/${type}/${id}/${url}`, src,
      contentType: type === "html" ? "text/html" : type === "pdf" ? "application/pdf" : "text/plain", format: type === "html" ? "html" : type, contentFormat: type === "html" ? "richText" : type };
  }
  const catalog = chapters!.map(chapter => {
    const next: Row = { ...optional(chapter), volume: !!chapter.volume };
    if (chapter.url) next.resource = descriptor(chapter.url);
    if (pageMap.has(chapter.id)) next.pages = pageMap.get(chapter.id)!.map(page => ({ ...optional(page), url: descriptor(page.url).resourceId, src: assetUrl(type, id, page.url) }));
    if (type === "audio" || type === "video") {
      const sources = streamMap.get(chapter.id) ?? [];
      const selected = progress.chapterId === chapter.id ? sources.find(source => source.label === progress.quality && source.url) : undefined;
      const selectedUrl = selected?.url ?? chapter.url;
      if (selectedUrl) next.media = { schemaVersion: 1, kind: "media", mediaType: type, src: assetUrl(type, id, selectedUrl), format: selected?.protocol ?? chapter.protocol ?? "direct",
        sources: sources.map(source => ({ label: source.label, src: source.url ? assetUrl(type, id, source.url) : undefined, format: source.protocol ?? "direct", selected: source.url === selectedUrl })) };
    }
    return next as ChapterResource;
  });
  if (type === "pdf" && row.url) for (let index = 0; index < (row.pageCount ?? 0); index++) catalog.push({ id: `page-${index + 1}`, title: String(index + 1), index,
    resource: descriptor(row.url) as ChapterResource["resource"], pdfPage: { schemaVersion: 1, kind: "pdfPage", src: assetUrl(type, id, row.url), pageIndex: index, defaultZoom: "page-fit" } });
  const base = { title: row.title, author: row.author ?? "", intro: row.intro ?? "", coverSrc: row.cover ? assetUrl(type, id, row.cover) : undefined };
  const overrides = optional({ title: row.titleOverride, author: row.authorOverride, intro: row.introOverride, coverSrc: row.coverOverride });
  if (overrides.coverSrc) overrides.coverSrc = assetUrl(type, id, overrides.coverSrc);
  return { ...optional(row), ...base, ...overrides, schemaVersion: 1, id, contentType: type, mediaType: type === "audio" || type === "video" ? type : undefined,
    kind: tags!.map(row => row.tag).join(" "), canChangeSource: !!row.sourceId && !!row.hasEngine, displayBase: base, displayOverrides: overrides,
    original: row.original ? JSON.parse(row.original) : undefined, progress, chapters: catalog, alternatives: alternatives!.map(optional) } as BookResource;
}

export async function readCategories(sourceId: string): Promise<DiscoveryCategoriesResource> {
  const [rows] = await tables([{ table: "discovery_categories", fields: "category_id:categoryId title kind style_json:style", where: "WHERE source_id=$1 AND private=0 ORDER BY position" }], [sourceId]);
  return { schemaVersion: 1, sourceId, categories: rows!.map(row => ({ ...optional(row), style: row.style ? JSON.parse(row.style) : undefined })) } as DiscoveryCategoriesResource;
}

export async function readTasks(): Promise<AppTask[]> {
  const [tasks, chapters, sources] = await tables([
    { table: "tasks", fields: "id kind status book_id:bookId keyword source_type:sourceType precision_search:precisionSearch page from_index:fromIndex total completed search_id:searchId result_json:result error created_at_ms:createdAtMs updated_at_ms:updatedAtMs", where: "ORDER BY position" },
    { table: "task_chapters", fields: "task_id:taskId chapter_id:id", where: "ORDER BY task_id,position" },
    { table: "task_sources", fields: "task_id:taskId source_id:id", where: "ORDER BY task_id,position" },
  ]);
  const index = (values: Row[]) => { const map = new Map<string, string[]>(); for (const row of values) { const ids = map.get(row.taskId) ?? []; ids.push(row.id); map.set(row.taskId, ids); } return map; };
  const chapterIds = index(chapters!), sourceIds = index(sources!);
  return tasks!.map(row => ({ ...booleans(optional(row), "precisionSearch"), result: row.result ? JSON.parse(row.result) : undefined,
    chapterIds: chapterIds.get(row.id), sourceIds: sourceIds.get(row.id) })) as AppTask[];
}

export async function readSearchHistory(): Promise<HistoryResource["search"]> {
  const [entries,ids]=await tables([
    {table:"search_history",fields:"query normalized_query:normalizedQuery usage first_use_time_ms:firstUseTimeMs last_use_time_ms:lastUseTimeMs",where:"ORDER BY last_use_time_ms DESC,usage DESC,normalized_query"},
    {table:"recorded_searches",fields:"id",where:"ORDER BY position"},
  ]);
  return {entries,processedSearchIds:ids!.map(row=>row.id)} as HistoryResource["search"];
}

export async function readReadingHistory():Promise<ReadingHistoryResource>{
  const [sessions,books,days,dayBooks,totals]=await tables([
    {table:"reading_sessions",fields:"book_id:bookId session_id:sessionId duration_ms:durationMs ended_at_ms:endedAtMs utc_offset_minutes:utcOffsetMinutes",where:"ORDER BY position"},
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
  const [sessions, books, days, dayBooks, entries, ids, bookmarks, totals] = await tables([
    { table: "reading_sessions", fields: "book_id:bookId session_id:sessionId duration_ms:durationMs ended_at_ms:endedAtMs utc_offset_minutes:utcOffsetMinutes", where: "ORDER BY position" },
    { table: "reading_books", fields: "book_id:bookId session_count:sessionCount total_duration_ms:totalDurationMs last_read_at_ms:lastReadAtMs", where: "ORDER BY last_read_at_ms DESC,book_id" },
    { table: "reading_days", fields: "day session_count:sessionCount total_duration_ms:totalDurationMs", where: "ORDER BY day DESC" },
    { table: "reading_day_books", fields: "day book_id:bookId session_count:sessionCount total_duration_ms:totalDurationMs", where: "ORDER BY day,position" },
    { table: "search_history", fields: "query normalized_query:normalizedQuery usage first_use_time_ms:firstUseTimeMs last_use_time_ms:lastUseTimeMs", where: "ORDER BY last_use_time_ms DESC,usage DESC,normalized_query" },
    { table: "recorded_searches", fields: "id", where: "ORDER BY position" },
    { table: "bookmarks", fields: "id book_id:bookId chapter_id:chapterId chapter_index:chapterIndex chapter_title:chapterTitle offset page_id:pageId anchor note orphaned created_at_ms:createdAtMs updated_at_ms:updatedAtMs", where: "ORDER BY book_id,position" },
    { table: "reading_totals", fields: "duration_ms:totalDurationMs sessions:totalSessions", where: "WHERE id=1" },
  ]);
  const grouped: Record<string, Row[]> = {};
  for (const bookmark of bookmarks!) (grouped[bookmark.bookId] ??= []).push(booleans(optional(bookmark), "orphaned"));
  const groupedDays = new Map<string, Row[]>();
  for (const row of dayBooks!) { const values = groupedDays.get(row.day) ?? []; values.push(row); groupedDays.set(row.day, values); }
  return { schemaVersion: 2, reading: { sessions, books, days: days!.map(day => ({ ...day, books: groupedDays.get(day.day) ?? [] })), ...totals![0] }, search: { entries, processedSearchIds: ids!.map(row => row.id) }, bookmarks: grouped } as HistoryResource;
}

/** 与写入接口使用相同的来源和 URL，直接读详情存储。 */
export async function readMoreInformation(sourceId: string, bookUrl: string): Promise<BookResource> {
  const rows = await database.select<Array<{ id: string }>>(`SELECT b.id FROM books b JOIN sources s ON s.id=b.source_id AND s.content_type=b.kind WHERE b.source_id=$1 AND coalesce(json_extract(b.engine_json,'$.bookUrl'),json_extract(b.engine_json,'$.url'))=$2 ORDER BY b.id LIMIT 1`, [sourceId, bookUrl]);
  if (!rows[0]) throw new Error("详情尚未保存");
  return readBook(rows[0].id);
}

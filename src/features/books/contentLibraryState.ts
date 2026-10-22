import { computed, nextTick, ref, watch, type WatchStopHandle } from "vue";
import { listen } from "@tauri-apps/api/event";
import { modifyDatabase } from "../../api/feDb";
import { readLibrary, assetUrl } from "../../api/database";
import { bookContentType } from "../../api/bookContentType";
import type { AppBootstrap, BookResource, ContentLibrary, ContentPreview, ContentType, ShelfDocument, ShelfSortKey, ShelfSortOrder } from "../../api/types";

export const contentTypes: ContentType[] = ["text", "html", "image", "audio", "video", "pdf"];
export const contentLibrary = ref<ContentLibrary>({});
const sortKeys: ShelfSortKey[] = ["updatedAt", "title", "author", "latestChapter", "progress", "custom"];
const savedSort = localStorage.getItem("librarySort") as ShelfSortKey;
export const librarySort = ref<ShelfSortKey>(sortKeys.includes(savedSort) ? savedSort : "custom");
export const librarySortOrder = ref<ShelfSortOrder>(localStorage.getItem("librarySortOrder") === "descending" ? "descending" : "ascending");
watch(librarySort, value => localStorage.setItem("librarySort", value));
watch(librarySortOrder, value => localStorage.setItem("librarySortOrder", value));
let initialized = false;
let requestId = 0;
let stopWatch: WatchStopHandle = () => {};
const progressUpdates = new Map<string, { updatedAtMs: number; read: number }>();
let refreshId = 0;

export function contentUrl(type: ContentType, id: string, name: string): string {
  if (name.startsWith("http://") || name.startsWith("https://")) return name;
  return assetUrl(type,id,name);
}

const libraryIndex = computed(() => {
  const entries = new Map<string, { type: ContentType; item: ContentPreview }>();
  for (const type of contentTypes) for (const item of contentLibrary.value[type] ?? []) entries.set(item.id, { type, item });
  return entries;
});

export function libraryType(id: string): ContentType | undefined { return libraryIndex.value.get(id)?.type; }

export function applyLibraryProgress(id: string, updatedAtMs: number, read: number): void {
  const entry = libraryIndex.value.get(id);
  if (!entry || (progressUpdates.get(id)?.updatedAtMs ?? 0) > updatedAtMs || (entry.item.readAt ?? 0) > Math.floor(updatedAtMs / 1000)) return;
  progressUpdates.set(id, { updatedAtMs, read });
  entry.item.readAt = Math.floor(updatedAtMs / 1000);
  entry.item.read = read;
}

export function clearLibraryProgress(id: string): void { progressUpdates.delete(id); }

export function libraryEntries(): Array<ContentPreview & { type: ContentType }> {
  return contentTypes.flatMap(type => (contentLibrary.value[type] ?? []).map(item => ({ ...item, type })));
}

/** This positional view is read-only. Every writer changes contentLibrary. */
export const libraryPositions = computed<ShelfDocument>(() => {
  const groups=contentLibrary.value.groups ?? [];
  const names=new Map<string,string[]>();
  for (const group of groups) for (const id of group.id) {
    const values=names.get(id) ?? []; values.push(group.name); names.set(id,values);
  }
  return { schemaVersion:1,sort:librarySort.value,sortOrder:librarySortOrder.value,groups:groups.map(group=>group.name),
    books:contentTypes.flatMap(type=>(contentLibrary.value[type] ?? []).map(item=>({id:item.id,groups:names.get(item.id) ?? []}))) };
});

function preview(book: BookResource): ContentPreview {
  const value: ContentPreview = { id: book.id, title: book.title };
  if (book.author) value.author = book.author;
  if (book.cover) value.cover = book.cover;
  else if (book.coverSrc && !book.coverSrc.includes("/r/")) value.cover = book.coverSrc;
  value.total = book.chapterCount;
  if (book.updatedAtMs) value.updatedAt = Math.floor(book.updatedAtMs / 1000);
  if (book.latestChapter) value.latest = book.latestChapter;
  if (book.progress?.updatedAtMs) {
    value.read = (book.progress.chapterIndex ?? 0) + 1;
    value.readAt = Math.floor(book.progress.updatedAtMs / 1000);
  }
  return value;
}

export function addLibraryContent(book: BookResource): void {
  if (libraryType(book.id)) return;
  const type = bookContentType(book);
  if (!type) throw new Error("内容类型缺失。");
  (contentLibrary.value[type] ??= []).push(preview(book));
}

export function removeLibraryContent(ids: Iterable<string>): void {
  const removed = new Set(ids);
  for (const type of contentTypes) {
    const items = contentLibrary.value[type];
    if (!items) continue;
    const next = items.filter(item => !removed.has(item.id));
    if (next.length) contentLibrary.value[type] = next;
    else delete contentLibrary.value[type];
  }
  for (const group of contentLibrary.value.groups ?? []) group.id = group.id.filter(id => !removed.has(id));
}

export function replaceLibraryContent(oldId: string, book: BookResource): void {
  const previousType = libraryType(oldId);
  if (!previousType) return;
  const groups = (contentLibrary.value.groups ?? []).map(group => ({ name: group.name,
    id: [...new Set(group.id.map(id => id === oldId ? book.id : id))] }));
  const index = contentLibrary.value[previousType]!.findIndex(item => item.id === oldId);
  removeLibraryContent([oldId, book.id]);
  const type = bookContentType(book);
  if (!type) throw new Error("内容类型缺失。");
  const items = contentLibrary.value[type] ??= [];
  items.splice(type === previousType ? Math.min(index, items.length) : items.length, 0, preview(book));
  contentLibrary.value.groups = groups;
}

export function setContentGroups(ids: string[], names: string[]): void {
  const selected = new Set(ids.filter(id => libraryType(id)));
  const groups = contentLibrary.value.groups ??= [];
  for (const name of new Set(names)) if (!groups.some(group => group.name === name)) groups.push({ name, id: [] });
  for (const group of groups) {
    group.id = group.id.filter(id => !selected.has(id));
    if (names.includes(group.name)) group.id.push(...selected);
  }
}

function installWatch(): void {
  stopWatch = watch(() => ({
    members: contentTypes.map(type => contentLibrary.value[type]?.map(item => item.id) ?? []),
    groups: (contentLibrary.value.groups ?? []).map(group => ({ name: group.name, id: [...group.id] })),
  }), () => {
    const id = ++requestId;
    // 只修改书架关系，书籍详情和章节缓存不受影响。
    void modifyDatabase([{ operation: "saveShelf", shelf: libraryPositions.value }], {
      onComplete: result => {
        if (result.error) { console.error(result.error); return; }
        if (id === requestId) void refreshContentLibrary().catch(console.error);
      },
    }).catch(console.error);
  }, { deep: true });
}

export async function refreshContentLibrary(): Promise<void> {
  if (!initialized) return;
  const currentRequest = requestId, currentRefresh = ++refreshId;
  const next = await readLibrary();
  await nextTick();
  if (currentRequest !== requestId || currentRefresh !== refreshId) return;
  for (const type of contentTypes) for (const item of next[type] ?? []) {
    const update = progressUpdates.get(item.id);
    if (update && Math.floor(update.updatedAtMs / 1000) >= (item.readAt ?? 0)) {
      item.readAt = Math.floor(update.updatedAtMs / 1000); item.read = update.read;
    }
  }
  stopWatch();
  contentLibrary.value = next;
  installWatch();
}

export async function loadContentLibrary(): Promise<void> {
  initialized = true;
  await listen("shelf-updated", () => {
    void refreshContentLibrary().catch(console.error);
  });
  await listen<AppBootstrap>("app-state-updated", () => {
    progressUpdates.clear();
    requestId++;
    void refreshContentLibrary().catch(console.error);
  });
  contentLibrary.value = await readLibrary();
  installWatch();
}

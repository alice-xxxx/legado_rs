import { computed, nextTick, ref, watch, type WatchStopHandle } from "vue";
import { listen } from "@tauri-apps/api/event";
import { command } from "../../api/ipc";
import { readResource } from "../../api/resources";
import { bookContentType } from "../../api/bookContentType";
import type { AppBootstrap, BookResource, ContentLibrary, ContentPreview, ContentType, ResourceDescriptor, ShelfDocument, ShelfSortKey, ShelfSortOrder } from "../../api/types";

export const contentTypes: ContentType[] = ["text", "html", "image", "audio", "video", "pdf"];
export const contentLibrary = ref<ContentLibrary>({});
const sortKeys: ShelfSortKey[] = ["updatedAt", "title", "author", "latestChapter", "progress", "custom"];
const savedSort = localStorage.getItem("librarySort") as ShelfSortKey;
export const librarySort = ref<ShelfSortKey>(sortKeys.includes(savedSort) ? savedSort : "custom");
export const librarySortOrder = ref<ShelfSortOrder>(localStorage.getItem("librarySortOrder") === "descending" ? "descending" : "ascending");
watch(librarySort, value => localStorage.setItem("librarySort", value));
watch(librarySortOrder, value => localStorage.setItem("librarySortOrder", value));
let resource: ResourceDescriptor;
let requestId = 0;
let stopWatch: WatchStopHandle = () => {};
const progressUpdates = new Map<string, { updatedAtMs: number; read: number }>();
let refreshId = 0;

export function contentUrl(type: ContentType, id: string, name = `${id}.json`): string {
  if (name.startsWith("http://") || name.startsWith("https://")) return name;
  const path = name.startsWith("resource://") ? name.slice("resource://".length)
    : `cache/${type}/${encodeURIComponent(id)}/${encodeURIComponent(name)}`;
  return resource ? new URL(path, resource.src).href : "";
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
export const libraryPositions = computed<ShelfDocument>(() => ({
  schemaVersion: 1,
  sort: librarySort.value,
  sortOrder: librarySortOrder.value,
  groups: (contentLibrary.value.groups ?? []).map(group => group.name),
  books: libraryEntries().map(item => ({ id: item.id,
    groups: (contentLibrary.value.groups ?? []).filter(group => group.id.includes(item.id)).map(group => group.name) })),
}));

function preview(book: BookResource): ContentPreview {
  const value: ContentPreview = { id: book.id, title: book.title };
  if (book.author) value.author = book.author;
  if (book.cover) value.cover = book.cover;
  else if (book.coverSrc && !book.coverSrc.startsWith(resource ? new URL(".", resource.src).href : "resource://")) value.cover = book.coverSrc;
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
    void command("save_shelf", { shelf: contentLibrary.value, requestId: ++requestId }).catch(console.error);
  }, { deep: true });
}

export async function refreshContentLibrary(): Promise<void> {
  if (!resource) return;
  const currentRequest = requestId, currentRefresh = ++refreshId;
  const next = await readResource<ContentLibrary>(resource);
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

export async function loadContentLibrary(descriptor: ResourceDescriptor): Promise<void> {
  resource = descriptor;
  contentLibrary.value = await readResource<ContentLibrary>(resource);
  await listen<{ requestId: number; resource: ResourceDescriptor }>("shelf-updated", ({ payload }) => {
    if (payload.requestId !== 0 && payload.requestId !== requestId) return;
    resource = payload.resource;
    void refreshContentLibrary().catch(console.error);
  });
  await listen<AppBootstrap>("app-state-updated", ({ payload }) => {
    progressUpdates.clear();
    requestId++;
    resource = payload.shelf;
    void refreshContentLibrary().catch(console.error);
  });
  installWatch();
}

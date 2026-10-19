import { nextTick, ref } from "vue";
import { listen } from "@tauri-apps/api/event";
import { modifyDatabase } from "../../api/feDb";
import { readHistory, readSearchHistory, readReadingHistory } from "../../api/database";
import type { AppBootstrap, BookmarkRecord, HistoryResource, ReadingHistoryResource } from "../../api/types";

export const history = ref<HistoryResource>({
  schemaVersion: 2,
  reading: { sessions: [], books: [], days: [], totalDurationMs: 0, totalSessions: 0 },
  search: { entries: [], processedSearchIds: [] }, bookmarks: {},
});
let requestId=0, restoreRevision=0, readRevision=0, searchRevision=0, readingRevision=0;

export async function removeHistoryBookmark(bookId:string,bookmarkId:string):Promise<void>{
  const restore=restoreRevision;
  ++requestId;
  await modifyDatabase([{ action: "delete", table: "bookmarks", filter: { book_id: bookId, id: bookmarkId } }]);
  if(restore!==restoreRevision)return;
  const next=(history.value.bookmarks[bookId] ?? []).filter(record=>record.id!==bookmarkId);
  if(next.length)history.value.bookmarks[bookId]=next;else delete history.value.bookmarks[bookId];
}
export async function saveHistoryBookmark(bookId:string,bookmark:BookmarkRecord):Promise<void>{
  const restore=restoreRevision;
  ++requestId;
  const existing = history.value.bookmarks[bookId] ?? [];
  const position = existing.findIndex(item => item.id === bookmark.id);
  await modifyDatabase([{ action: "upsert", table: "bookmarks", keys: ["id"], values: {
    id: bookmark.id, book_id: bookId, position: position < 0 ? existing.length : position,
    chapter_id: bookmark.chapterId, chapter_index: bookmark.chapterIndex, chapter_title: bookmark.chapterTitle,
    offset: bookmark.offset ?? null, page_id: bookmark.pageId ?? null, anchor: bookmark.anchor ?? null,
    note: bookmark.note, orphaned: bookmark.orphaned ?? false, created_at_ms: bookmark.createdAtMs, updated_at_ms: bookmark.updatedAtMs,
  } }]);
  if(restore!==restoreRevision)return;
  const records=history.value.bookmarks[bookId] ??= [];
  const index=records.findIndex(record=>record.id===bookmark.id);
  if(index<0)records.push(bookmark);else records[index]=bookmark;
}
export async function recordReadingSession(session:ReadingHistoryResource["sessions"][number]):Promise<void>{
  const restore=restoreRevision;
  ++requestId;
  await modifyDatabase([{ action: "recordSession", session }]);
  const revision=++readingRevision;
  const next=await readReadingHistory();
  if(restore===restoreRevision && revision===readingRevision)history.value.reading=next;
}
export async function removeSearchHistory(query?:string):Promise<void>{
  const restore=restoreRevision;
  ++requestId;
  await modifyDatabase([{ action: "delete", table: "search_history", filter: query === undefined ? {} : { query } }]);
  if(restore===restoreRevision)await updateHistory("search");
}

async function updateHistory(part?:string):Promise<void>{
  const restore=restoreRevision;
  if(part==="search"){
    const revision=++searchRevision;
    const next=await readSearchHistory();
    if(restore===restoreRevision && revision===searchRevision)history.value.search=next;
    return;
  }
  const revision=++readRevision, mutation=requestId;
  const next=await readHistory();
  await nextTick();
  if(restore!==restoreRevision || revision!==readRevision || mutation!==requestId)return;
  history.value=next;
}
export async function loadHistory():Promise<void>{
  history.value=await readHistory();
  await listen<AppBootstrap>("app-state-updated",()=>{
    restoreRevision++;requestId++;readingRevision++;searchRevision++;
    void updateHistory().catch(console.error);
  });
  await listen<{requestId?:number;part?:string}>("history-updated",({payload})=>{
    // 搜索通知没有 requestId，收到通知后直接刷新对应的历史部分。
    if(payload.requestId===undefined||payload.requestId===0)void updateHistory(payload.part).catch(console.error);
  });
}

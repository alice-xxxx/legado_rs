import { nextTick, ref } from "vue";
import { listen } from "@tauri-apps/api/event";
import { modifyDatabase } from "../../api/feDb";
import { readHistory, readSearchHistory, readReadingHistory } from "../../api/database";
import type { AppBootstrap, BookmarkRecord, HistoryResource, ReadingHistoryResource } from "../../api/types";

export const history = ref<HistoryResource>({
  schemaVersion: 2,
  reading: { sessions: [], books: [], days: [], totalDurationMs: 0, totalSessions: 0 },
  search: { entries: [] }, bookmarks: {},
});
let requestId=0, restoreRevision=0, readRevision=0, searchRevision=0, readingRevision=0;

export async function removeHistoryBookmark(bookId:string,bookmarkId:string):Promise<void>{
  const restore=restoreRevision;
  ++requestId;
  await modifyDatabase([{ operation: "deleteBookmark", bookId, id: bookmarkId }], { wait: true });
  if(restore!==restoreRevision)return;
  const next=(history.value.bookmarks[bookId] ?? []).filter(record=>record.id!==bookmarkId);
  if(next.length)history.value.bookmarks[bookId]=next;else delete history.value.bookmarks[bookId];
}
export async function saveHistoryBookmark(bookId:string,bookmark:BookmarkRecord):Promise<void>{
  const restore=restoreRevision;
  ++requestId;
  await modifyDatabase([{ operation: "saveBookmark", bookId, bookmark }], { wait: true });
  if(restore!==restoreRevision)return;
  const records=history.value.bookmarks[bookId] ??= [];
  const index=records.findIndex(record=>record.id===bookmark.id);
  if(index<0)records.push(bookmark);else records[index]=bookmark;
}
export async function recordReadingSession(session:ReadingHistoryResource["sessions"][number]):Promise<void>{
  const restore=restoreRevision;
  ++requestId;
  await modifyDatabase([{ operation: "recordSession", session }], { wait: true });
  const revision=++readingRevision;
  const next=await readReadingHistory();
  if(restore===restoreRevision && revision===readingRevision)history.value.reading=next;
}
export async function removeSearchHistory(query?:string):Promise<void>{
  const restore=restoreRevision;
  ++requestId;
  await modifyDatabase([{ operation: "clearSearchHistory", query }], { wait: true });
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
  await listen<{part?:string}>("history-updated",({payload})=>{
    void updateHistory(payload.part).catch(console.error);
  });
}

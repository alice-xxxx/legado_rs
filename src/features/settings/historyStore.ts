import { nextTick, ref } from "vue";
import { listen } from "@tauri-apps/api/event";
import { mutateHistory } from "../../api/history";
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
  await mutateHistory("removeBookmark",{bookId,bookmarkId},++requestId);
  if(restore!==restoreRevision)return;
  const next=(history.value.bookmarks[bookId] ?? []).filter(record=>record.id!==bookmarkId);
  if(next.length)history.value.bookmarks[bookId]=next;else delete history.value.bookmarks[bookId];
}
export async function saveHistoryBookmark(bookId:string,bookmark:BookmarkRecord):Promise<void>{
  const restore=restoreRevision;
  await mutateHistory("saveBookmark",{bookId,bookmark},++requestId);
  if(restore!==restoreRevision)return;
  const records=history.value.bookmarks[bookId] ??= [];
  const index=records.findIndex(record=>record.id===bookmark.id);
  if(index<0)records.push(bookmark);else records[index]=bookmark;
}
export async function recordReadingSession(session:ReadingHistoryResource["sessions"][number]):Promise<void>{
  const restore=restoreRevision;
  await mutateHistory("recordSession",session,++requestId);
  const revision=++readingRevision;
  const next=await readReadingHistory();
  if(restore===restoreRevision && revision===readingRevision)history.value.reading=next;
}
export async function removeSearchHistory(query?:string):Promise<void>{
  const restore=restoreRevision;
  await mutateHistory(query===undefined?"clearSearch":"removeSearch",{query},++requestId);
  if(restore!==restoreRevision)return;
  history.value.search.entries=query===undefined?[]:history.value.search.entries.filter(entry=>entry.query!==query);
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
  await listen<{requestId:number;part?:string}>("history-updated",({payload})=>{
    if(payload.requestId===0)void updateHistory(payload.part).catch(console.error);
  });
}

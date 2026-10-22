import { Channel } from "@tauri-apps/api/core";
import { command } from "./ipc";
import type { BookmarkRecord, DisplayReplacementRule, ReadingHistoryResource, ReadingProgress, ShelfDocument, TxtTocRule } from "./types";

export type SourceInput = { id?: string; definition?: string; enabled?: boolean; group?: string | null; removed?: boolean };
export type DatabaseOperation =
  | { operation: "saveSources"; sources: SourceInput[] }
  | { operation: "saveBooks"; books: Array<{ sourceId: string; bookUrl: string; title: string; author?: string; cover?: string; latest?: string }> }
  | { operation: "saveBook"; bookId: string; changes: { title?: string | null; author?: string | null; intro?: string | null; cover?: string | null; tags?: string[]; alternatives?: string[]; pageCount?: number } }
  | { operation: "useChapterContent"; bookId: string; chapterId: string; contentId: string }
  | { operation: "saveProgress"; bookId: string; progress: ReadingProgress }
  | { operation: "resetProgress"; bookId: string }
  | { operation: "saveShelf"; shelf: ShelfDocument }
  | { operation: "saveReplacementRules"; rules: DisplayReplacementRule[] }
  | { operation: "deleteReplacementRule" | "deleteTxtTocRule" | "deleteHttpTts"; id: string }
  | { operation: "saveTxtTocRules"; rules: TxtTocRule[] }
  | { operation: "saveHttpTts"; configs: Array<{ id: string; name: string; enabled: boolean; definition: string }> }
  | { operation: "saveBookmark"; bookId: string; bookmark: BookmarkRecord }
  | { operation: "deleteBookmark"; bookId: string; id: string }
  | { operation: "recordSession"; session: ReadingHistoryResource["sessions"][number] }
  | { operation: "clearSearchHistory"; query?: string };

export interface WriteCompletion { error: string | null }
export interface WriteOptions { wait?: boolean; onComplete?: (result: WriteCompletion) => void; prepareCurrent?: boolean }

/** 默认只等待入队；wait 等待提交，onComplete 接收完成通知。 */
export async function modifyDatabase(operations: DatabaseOperation[], options: WriteOptions = {}): Promise<void> {
  const channel = new Channel<WriteCompletion>();
  let finish!: (result: WriteCompletion) => void;
  const committed = new Promise<WriteCompletion>(resolve => { finish = resolve; });
  channel.onmessage = result => {
    finish(result);
    try { options.onComplete?.(result); } catch (error) { console.error(error); }
    if (result.error && !options.wait && !options.onComplete) console.error("数据库写入失败", result.error);
  };
  await command<void>("fe_db", { operations, completed: channel, prepareCurrent: options.prepareCurrent });
  if (options.wait) {
    const result = await committed;
    if (result.error) throw new Error(result.error);
  }
}

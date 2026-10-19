import { command } from "./ipc";
import { listen } from "@tauri-apps/api/event";
import type { SearchPrecision, SearchUpdate } from "./types";

// 同一窗口按调用顺序搜索，使通知直接交给当前调用者，无需回传参数或 request_id。
let pending: Promise<unknown> = Promise.resolve();

export function search(sourceIds: string[], keyword: string, page: number,
  precision: SearchPrecision, onResults: (update: SearchUpdate) => void): Promise<string> {
  const run = pending.then(async () => {
    let searchId: string | undefined;
    const unlisten = await listen<SearchUpdate>("search-updated", ({ payload }) => {
      if (!payload.sourceId) searchId = payload.searchId;
      if (payload.searchId === searchId) onResults(payload);
    });
    try {
      return await command<string>("search", { sourceIds, keyword, page, precision });
    } finally {
      unlisten();
    }
  });
  pending = run.catch(() => {});
  return run;
}

// 直接取消搜索会话，不经过后台任务中心。
export const stopSearch = (searchId: string) => command<void>("stop_search", { searchId });
export const releaseSearchResults = (searchId: string) => command<void>("release_search_results", { searchId });

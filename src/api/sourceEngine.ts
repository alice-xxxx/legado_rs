import { invoke } from "@tauri-apps/api/core";

export type SourceEngineOperation = "search" | "bookInfo" | "chapters" | "content";

export interface SourceEngineRequest {
  operation: SourceEngineOperation;
  source: Record<string, unknown>;
  keyword?: string;
  page?: number;
  book?: Record<string, unknown>;
  chapter?: Record<string, unknown>;
  nextChapterUrl?: string;
}

/** UI 只把操作交给桌面 Rust；书源 HTTP、规则执行和 cookie 存储都在 Rust/KMP 路径中完成。 */
export function executeSourceEngine(request: SourceEngineRequest): Promise<unknown> {
  return invoke("execute_source_engine", { request });
}

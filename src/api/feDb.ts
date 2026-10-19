import { command } from "./ipc";
import type { ReadingHistoryResource } from "./types";

type Fields = Record<string, unknown>;
export type DatabaseChange =
  | { action: "recordSession"; session: ReadingHistoryResource["sessions"][number] }
  | { action: "insert"; table: string; values: Fields }
  | { action: "upsert"; table: string; values: Fields; keys: string[] }
  | { action: "update"; table: string; values: Fields; filter: Fields }
  | { action: "delete"; table: string; filter: Fields };

/** 与 Rust 共用参数化写入；多项修改在一个事务内完成。 */
export const modifyDatabase = (changes: DatabaseChange[]) => command<void>("fe_db", { changes });

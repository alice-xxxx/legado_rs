import { command } from "./ipc";
import type { HistoryResource, ResourceDescriptor } from "./types";

export interface SaveHistoryResponse {
  requestId: number;
  resource: ResourceDescriptor;
}

export const saveHistory = (history: HistoryResource, requestId: number) =>
  command<SaveHistoryResponse>("save_history", { history, requestId });

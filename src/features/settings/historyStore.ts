import { nextTick, ref, watch, type WatchStopHandle } from "vue";
import { listen } from "@tauri-apps/api/event";
import { saveHistory } from "../../api/history";
import { readResource } from "../../api/resources";
import type { AppBootstrap, HistoryResource, ResourceDescriptor } from "../../api/types";

export const history = ref<HistoryResource>({
  schemaVersion: 2,
  reading: { sessions: [], books: [], days: [], totalDurationMs: 0, totalSessions: 0 },
  search: { entries: [], processedSearchIds: [] },
  bookmarks: {},
});

export function removeHistoryBookmark(bookId: string, bookmarkId: string): void {
  const records = history.value.bookmarks[bookId];
  if (!records) return;
  const next = records.filter((record) => record.id !== bookmarkId);
  if (next.length) history.value.bookmarks[bookId] = next;
  else delete history.value.bookmarks[bookId];
}

type HistoryUpdatedEvent = { requestId: number; resource: ResourceDescriptor };
let requestId = 0;
let stopWatch: WatchStopHandle = () => {};

function watchHistory(): void {
  stopWatch = watch(history, () => {
    void saveHistory(history.value, ++requestId).catch(console.error);
  }, { deep: true });
}

async function updateHistory({ requestId: eventRequestId, resource }: HistoryUpdatedEvent): Promise<void> {
  if (eventRequestId !== 0 && eventRequestId !== requestId) return;
  const id = requestId;
  const next = await readResource<HistoryResource>(resource);
  await nextTick();
  if (id !== requestId) return;
  stopWatch();
  history.value = next;
  watchHistory();
}

export async function loadHistory(descriptor: ResourceDescriptor): Promise<void> {
  history.value = await readResource<HistoryResource>(descriptor);
  await listen<AppBootstrap>("app-state-updated", ({ payload }) => { requestId++; void updateHistory({ requestId: 0, resource: payload.history }).catch(console.error); });
  await listen<HistoryUpdatedEvent>("history-updated", ({ payload }) => {
    void updateHistory(payload).catch(console.error);
  });
  watchHistory();
}

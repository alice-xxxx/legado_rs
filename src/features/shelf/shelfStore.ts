import { nextTick, ref, watch, type WatchStopHandle } from "vue";
import { listen } from "@tauri-apps/api/event";
import { command } from "../../api/ipc";
import { readResource } from "../../api/resources";
import type { AppBootstrap, ResourceDescriptor, ShelfDocument } from "../../api/types";

export const shelf = ref<ShelfDocument>({ schemaVersion: 1, books: [], groups: [] });
type ShelfUpdatedEvent = { requestId: number; resource: ResourceDescriptor };
let requestId = 0;
let stopWatch: WatchStopHandle = () => {};

function watchShelf(): void {
  stopWatch = watch(shelf, () => {
    void command("save_shelf", { shelf: shelf.value, requestId: ++requestId }).catch(console.error);
  }, { deep: true });
}

async function updateShelf({ requestId: eventRequestId, resource }: ShelfUpdatedEvent): Promise<void> {
  if (eventRequestId !== 0 && eventRequestId !== requestId) return;
  const id = requestId;
  const next = await readResource<ShelfDocument>(resource);
  await nextTick();
  if (id !== requestId) return;
  stopWatch();
  shelf.value = next;
  watchShelf();
}

export async function loadShelf(descriptor: ResourceDescriptor): Promise<void> {
  shelf.value = await readResource<ShelfDocument>(descriptor);
  await listen<AppBootstrap>("app-state-updated", ({ payload }) => {
    requestId++;
    void updateShelf({ requestId: 0, resource: payload.shelf }).catch(console.error);
  });
  await listen<ShelfUpdatedEvent>("shelf-updated", ({ payload }) => {
    void updateShelf(payload).catch(console.error);
  });
  watchShelf();
}

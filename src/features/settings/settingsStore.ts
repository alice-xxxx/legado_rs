import { nextTick, ref, watch, type WatchStopHandle } from "vue";
import { listen } from "@tauri-apps/api/event";
import { readResource } from "../../api/resources";
import { saveSettings } from "../../api/settings";
import type { AppBootstrap, AppSettingsResource, ResourceDescriptor } from "../../api/types";
import { defaultSettings } from "./settingsDefaults";
export const settings = ref<AppSettingsResource>(defaultSettings);

type SettingsUpdatedEvent = { requestId: number; resource: ResourceDescriptor };
let requestId = 0, stopWatch: WatchStopHandle = () => { };

function watchSettings() {
  stopWatch = watch(settings, () => { void saveSettings(settings.value, ++requestId).catch(console.error); }, { deep: true });
}

async function updateSettings({ requestId: eventRequestId, resource }: SettingsUpdatedEvent) {
  if (eventRequestId !== 0 && eventRequestId !== requestId) {
    return;
  }
  const id = requestId;
  const next = await readResource<AppSettingsResource>(resource);
  await nextTick();
  if (id !== requestId) return;

  stopWatch();
  settings.value = next;
  watchSettings();
}

export async function loadSettings(descriptor: ResourceDescriptor): Promise<void> {
  settings.value = await readResource<AppSettingsResource>(descriptor);
  await listen<AppBootstrap>("app-state-updated", ({ payload }) => { requestId++; void updateSettings({ requestId: 0, resource: payload.settings }).catch(console.error); });
  await listen<SettingsUpdatedEvent>("settings-updated", ({ payload }) => { void updateSettings(payload).catch(console.error); });
  watchSettings();
}

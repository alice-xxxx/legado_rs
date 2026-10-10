<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { appBootstrap } from "../../api/core";
import { readResource } from "../../api/resources";
import { saveSettings } from "../../api/settings";
import type { AppBootstrap, AppSettingsResource, ResourceDescriptor } from "../../api/types";
import { defaultSettings, normalizeSettings } from "./settingsDefaults";
import { backupBusy } from "./backupState";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { notify } from "../../app/notifications";
import SettingsDetailHeader from "./SettingsDetailHeader.vue";

const settings = ref<AppSettingsResource>(structuredClone(defaultSettings));
const loading = ref(true);
const loadError = ref("");
const saveBusy = ref(false);
const saveError = ref("");
const operationBlocked = computed(() => backupBusy.value !== null || shelfBatchRecoveryRequired.value);
const controlsDisabled = computed(() => operationBlocked.value || loading.value || !!loadError.value || saveBusy.value);

let disposed = false;
let restoreRevision = 0;
let readRevision = 0;
let settingsEventRevision = 0;
let changeRevision = 0;
const unlisteners: UnlistenFn[] = [];

function errorText(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason);
}

async function applySettingsDescriptor(descriptor: ResourceDescriptor): Promise<void> {
  const revision = ++readRevision;
  const restoreAtStart = restoreRevision;
  const eventAtStart = ++settingsEventRevision;
  const changeAtStart = changeRevision;
  loading.value = true;
  loadError.value = "";
  try {
    const next = normalizeSettings(await readResource<AppSettingsResource>(descriptor));
    if (disposed || revision !== readRevision || restoreAtStart !== restoreRevision || eventAtStart !== settingsEventRevision) return;
    if (changeAtStart === changeRevision) settings.value = next;
  } catch (reason) {
    if (!disposed && revision === readRevision && restoreAtStart === restoreRevision && eventAtStart === settingsEventRevision) {
      loadError.value = errorText(reason);
    }
  } finally {
    if (!disposed && revision === readRevision && restoreAtStart === restoreRevision && eventAtStart === settingsEventRevision) {
      loading.value = false;
    }
  }
}

async function loadSettings(snapshot?: AppBootstrap): Promise<void> {
  const revision = ++readRevision;
  const restoreAtStart = restoreRevision;
  const eventAtStart = settingsEventRevision;
  const changeAtStart = changeRevision;
  loading.value = true;
  loadError.value = "";
  try {
    const current = snapshot ?? await appBootstrap();
    const next = normalizeSettings(await readResource<AppSettingsResource>(current.settings));
    if (disposed || revision !== readRevision || restoreAtStart !== restoreRevision || eventAtStart !== settingsEventRevision) return;
    if (changeAtStart === changeRevision) settings.value = next;
  } catch (reason) {
    if (!disposed && revision === readRevision && restoreAtStart === restoreRevision && eventAtStart === settingsEventRevision) {
      loadError.value = errorText(reason);
    }
  } finally {
    if (!disposed && revision === readRevision && restoreAtStart === restoreRevision && eventAtStart === settingsEventRevision) {
      loading.value = false;
    }
  }
}

async function persistSettings(): Promise<void> {
  if (operationBlocked.value || loading.value || !!loadError.value || saveBusy.value) return;
  const restoreAtStart = restoreRevision;
  const eventAtStart = settingsEventRevision;
  const changeAtStart = changeRevision;
  const next = normalizeSettings(settings.value);
  saveBusy.value = true;
  saveError.value = "";
  try {
    const descriptor = await saveSettings(next);
    const persisted = normalizeSettings(await readResource<AppSettingsResource>(descriptor));
    if (disposed || restoreAtStart !== restoreRevision) return;
    if (eventAtStart === settingsEventRevision && changeAtStart === changeRevision) settings.value = persisted;
    notify("网络请求设置已保存。");
  } catch (reason) {
    if (!disposed && restoreAtStart === restoreRevision) {
      saveError.value = errorText(reason);
      notify(`网络请求设置保存失败：${saveError.value}`, "error");
    }
  } finally {
    if (!disposed && restoreAtStart === restoreRevision) saveBusy.value = false;
  }
}

function changeTimeout(seconds: number): void {
  if (controlsDisabled.value) return;
  changeRevision += 1;
  settings.value = normalizeSettings({ ...settings.value, sourceHttpTimeoutSeconds: seconds });
  saveError.value = "";
  void persistSettings();
}

function retrySave(): void {
  saveError.value = "";
  void persistSettings();
}

function handleRestore(snapshot: AppBootstrap): void {
  restoreRevision += 1;
  readRevision += 1;
  settingsEventRevision += 1;
  changeRevision += 1;
  saveBusy.value = false;
  saveError.value = "";
  void loadSettings(snapshot);
}

onMounted(async () => {
  try {
    const unlistenSettings = await listen<ResourceDescriptor>("settings-updated", (event) => {
      void applySettingsDescriptor(event.payload);
    });
    if (disposed) { unlistenSettings(); return; }
    unlisteners.push(unlistenSettings);
  } catch (reason) {
    if (!disposed) notify(`设置更新监听注册失败：${errorText(reason)}`, "error");
  }

  if (disposed) return;
  try {
    const unlistenRestore = await listen<AppBootstrap>("app-state-updated", (event) => {
      handleRestore(event.payload);
    });
    if (disposed) { unlistenRestore(); return; }
    unlisteners.push(unlistenRestore);
  } catch (reason) {
    if (!disposed) notify(`设置恢复监听注册失败：${errorText(reason)}`, "error");
  }

  if (disposed) return;
  await loadSettings();
});

onBeforeUnmount(() => {
  disposed = true;
  restoreRevision += 1;
  readRevision += 1;
  for (const unlisten of unlisteners) unlisten();
});
</script>

<template>
  <section class="settings-detail-page">
    <SettingsDetailHeader title="网络请求" />
    <p v-if="loading" class="settings-detail-state" role="status">正在载入网络请求设置…</p>
    <div v-else-if="loadError" class="settings-detail-state settings-detail-error" role="alert">
      <span>网络请求设置载入失败：{{ loadError }}</span>
      <button type="button" class="text-button" :disabled="loading" @click="loadSettings()">重试</button>
    </div>
    <div v-else class="settings-detail-content">
      <section class="settings-detail-panel">
        <header class="settings-detail-section-header"><strong>来源网络请求</strong></header>
        <div class="setting-control">
          <div>
            <label for="source-http-timeout">最长时限</label>
            <small>连接或读取超时后结束请求</small>
          </div>
          <select
            id="source-http-timeout"
            :value="settings.sourceHttpTimeoutSeconds"
            :disabled="controlsDisabled"
            @change="changeTimeout(Number(($event.target as HTMLSelectElement).value))"
          >
            <option :value="15">15 秒</option>
            <option :value="30">30 秒</option>
            <option :value="60">60 秒</option>
            <option :value="120">120 秒</option>
          </select>
        </div>
        <p v-if="saveError" class="settings-detail-state settings-detail-error" role="alert">
          <span>保存失败：{{ saveError }}</span>
          <button type="button" class="text-button" :disabled="saveBusy || operationBlocked" @click="retrySave">重试</button>
        </p>
        <p v-else-if="saveBusy" class="settings-detail-state" role="status">正在保存…</p>
      </section>
    </div>
  </section>
</template>

<style scoped>
.settings-detail-page {
  width: min(820px, 100%);
  min-height: 100dvh;
  margin: 0 auto;
  padding: 10px 0 50px;
  color: var(--app-text);
}

.settings-detail-content { display: grid; gap: 18px; }
.settings-detail-panel { padding: 0; border: 0; border-radius: 0; background: transparent; }
.settings-detail-section-header {
  display: grid;
  gap: 4px;
  margin: 0 0 9px;
  padding: 0 2px 12px;
  border-bottom: 1px solid var(--app-line);
}
.settings-detail-section-header strong { color: #373a45; font-size: 13px; }
.setting-control {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  min-height: 68px;
  padding: 8px 4px;
  border-bottom: 1px solid #e8eaf0;
}
.setting-control > div { display: grid; gap: 4px; min-width: 0; }
.setting-control label { color: #383a45; font-size: 13px; font-weight: 650; }
.setting-control small,
.settings-detail-state { color: #9196a1; font-size: 11px; line-height: 1.5; }
.setting-control select {
  min-width: 105px;
  min-height: 36px;
  padding: 5px;
  border: 1px solid #e4e5ec;
  border-radius: 8px;
  background: #fff;
}
.settings-detail-state { display: flex; align-items: center; gap: 10px; margin: 12px 2px; }
.settings-detail-error { color: #a23c3c; }
.text-button { border: 0; padding: 4px 0; color: #4f61b2; background: transparent; cursor: pointer; }

@media (max-width: 640px) {
  .settings-detail-content { gap: 22px; }
  .setting-control { min-height: 65px; align-items: center; }
  .setting-control > div { flex: 1; max-width: 60%; }
}
</style>

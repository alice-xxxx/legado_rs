<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { onBeforeRouteLeave } from "vue-router";
import { appBootstrap } from "../../api/core";
import { readResource } from "../../api/resources";
import type { AppBootstrap, AppSettingsResource, HttpTtsConfigEditorFields, HttpTtsConfigMetadata, ResourceDescriptor } from "../../api/types";
import { useHttpTtsConfigActions } from "./useHttpTtsConfigActions";
import { defaultSettings, normalizeSettings } from "./settingsDefaults";
import ManagedListRow from "../../ui/ManagedListRow.vue";
import ManagedListToolbar from "../../ui/ManagedListToolbar.vue";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import BackButton from "../../ui/BackButton.vue";
import { useOutsidePointerDismiss } from "../../ui/useOutsidePointerDismiss";
import router, { routeNames } from "../../router";
import { backupBusy } from "./backupState";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { notify } from "../../app/notifications";
import { selectReaderHttpTtsConfig } from "../reader/readerTtsConfigService";

const settings = ref<AppSettingsResource>(structuredClone(defaultSettings));
const configs = ref<HttpTtsConfigMetadata[]>([]);
const configResource = ref<ResourceDescriptor | null>(null);
const listBusy = ref(true);
const importBusy = ref(false);
const editBusy = ref(false);
const deleteBusyId = ref<string | null>(null);
const error = ref("");
const settingsLoading = ref(true);
const settingsReady = ref(false);
const blocked = computed(() => backupBusy.value !== null || shelfBatchRecoveryRequired.value
  || settingsLoading.value || !settingsReady.value);
const selectedConfigId = computed(() => settings.value.reader.httpTtsConfigId ?? "");
let disposed = false;
let restoreRevision = 0;
let stateReadRevision = 0;
let settingsUpdateRevision = 0;
let configResourceRevision = 0;
let settingsChangeRevision = 0;
const unlisteners: UnlistenFn[] = [];

function errorText(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason);
}

function selectConfig(id: string): void {
  settingsChangeRevision += 1;
  settings.value = {
    ...settings.value,
    reader: { ...settings.value.reader, httpTtsConfigId: id || null },
  };
  selectReaderHttpTtsConfig(id);
}

const httpTtsConfigActions = useHttpTtsConfigActions({
  configs,
  busy: listBusy,
  error,
  importBusy,
  editBusy,
  deleteBusyId,
  resource: configResource,
  settings,
  selectConfig,
  getRestoreRevision: () => restoreRevision,
  notify,
  errorText,
});

async function loadSettingsDescriptor(descriptor: ResourceDescriptor): Promise<void> {
  const revision = ++settingsUpdateRevision;
  const restoreAtStart = restoreRevision;
  const selectionAtStart = settingsChangeRevision;
  try {
    const next = normalizeSettings(await readResource<AppSettingsResource>(descriptor));
    if (disposed || revision !== settingsUpdateRevision || restoreAtStart !== restoreRevision) return;
    if (selectionAtStart === settingsChangeRevision) settings.value = next;
    settingsReady.value = true;
  } catch (reason) {
    if (!disposed && revision === settingsUpdateRevision && restoreAtStart === restoreRevision) {
      error.value = "Settings resource read failed: " + errorText(reason);
    }
  }
}

async function loadPageData(snapshot?: AppBootstrap): Promise<void> {
  const revision = ++stateReadRevision;
  const restoreAtStart = restoreRevision;
  const settingsRevisionAtStart = settingsUpdateRevision;
  const configRevisionAtStart = configResourceRevision;
  const selectionAtStart = settingsChangeRevision;
  settingsLoading.value = true;
  error.value = "";
  try {
    const current = snapshot ?? await appBootstrap();
    const nextSettings = normalizeSettings(await readResource<AppSettingsResource>(current.settings));
    if (disposed || revision !== stateReadRevision || restoreAtStart !== restoreRevision) return;
    if (settingsRevisionAtStart === settingsUpdateRevision && selectionAtStart === settingsChangeRevision) {
      settings.value = nextSettings;
      settingsReady.value = true;
    }
    if (configRevisionAtStart === configResourceRevision) {
      configResource.value = current.httpTtsConfigs;
      await httpTtsConfigActions.refreshHttpTtsConfigs(current.httpTtsConfigs);
    }
  } catch (reason) {
    if (!disposed && revision === stateReadRevision && restoreAtStart === restoreRevision) {
      error.value = "HTTP TTS settings read failed: " + errorText(reason);
      listBusy.value = false;
    }
  } finally {
    if (!disposed && revision === stateReadRevision && restoreAtStart === restoreRevision) settingsLoading.value = false;
  }
}

async function refreshPageData(): Promise<void> {
  await loadPageData();
}

function handleResourceUpdated(payload: { kind: string; resource: ResourceDescriptor }): void {
  if (payload.kind !== "httpTtsConfigs" || disposed) return;
  configResourceRevision += 1;
  configResource.value = payload.resource;
  void httpTtsConfigActions.refreshHttpTtsConfigs(payload.resource);
}

function handleRestore(snapshot: AppBootstrap): void {
  restoreRevision += 1;
  stateReadRevision += 1;
  settingsUpdateRevision += 1;
  configResourceRevision += 1;
  settingsChangeRevision += 1;
  settingsReady.value = false;
  httpTtsConfigActions.resetForRestore();
  void loadPageData(snapshot);
}

function returnToAdvancedSettings(): void {
  void router.push({ name: routeNames.settingsOther });
}

const selectedConfig = computed(() => configs.value.find((item) => item.id === selectedConfigId.value));
const openConfigMenu = computed(() => configs.value.find((item) => item.id === openConfigMenuId.value));
const filteredConfigs = computed(() => {
  const query = configQuery.value.trim().toLocaleLowerCase();
  return configs.value.filter((config) => {
    const matchesQuery = !query || config.name.toLocaleLowerCase().includes(query);
    const matchesStatus = enabledFilter.value === "all"
      || (enabledFilter.value === "enabled" ? config.enabled : !config.enabled);
    return matchesQuery && matchesStatus;
  });
});
const selectedConfigs = computed(() => configs.value.filter((config) => selectedConfigIds.value.includes(config.id)));
const allVisibleSelected = computed(() => filteredConfigs.value.length > 0
  && filteredConfigs.value.every((config) => selectedConfigIds.value.includes(config.id)));
const editingConfig = computed(() => configs.value.find((item) => item.id === editingConfigId.value));
const creatingConfig = computed(() => editorOpen.value && !editingConfigId.value);
const importUrl = ref("");
const configQuery = ref("");
const enabledFilter = ref<"all" | "enabled" | "disabled">("all");
const filterMenuOpen = ref(false);
const actionsMenuOpen = ref(false);
const batchMode = ref(false);
const selectedConfigIds = ref<string[]>([]);
const addMenuOpen = ref(false);
const openConfigMenuId = ref<string | null>(null);
const configMenuPosition = ref({ top: 0, left: 0 });
const filterMenuPosition = ref({ top: 0, left: 0 });
const actionsMenuPosition = ref({ top: 0, left: 0 });
const importUrlOpen = ref(false);
const importUrlError = ref("");
watch(importUrl, () => { importUrlError.value = ""; });
const pendingConfigId = ref<string | null>(null);
const pendingSelectionId = ref<string | null>(null);
const editorError = ref("");
const editorSuccess = ref("");
const pendingDeleteIds = ref<string[]>([]);
const pendingDeleteConfigs = computed(() => configs.value.filter((item) => pendingDeleteIds.value.includes(item.id)));
const pendingCreate = ref(false);
const isMounted = ref(false);
const initialDesktopOpened = ref(false);
const importUrlBusy = ref(false);
const editorOpen = ref(false);
const initialEditorFields = ref("");
const discardPromptOpen = ref(false);
const editorLoading = ref(false);
const editingConfigId = ref("");
const fields = reactive<HttpTtsConfigEditorFields>(emptyFields());
const editorDirty = computed(() => editorOpen.value && JSON.stringify(fields) !== initialEditorFields.value);
let pendingNavigation: ((allow: boolean) => void) | null = null;
onBeforeRouteLeave(() => {
  if (!editorOpen.value) return true;
  if (editBusy.value || editorLoading.value) return false;
  if (!editorDirty.value) return true;
  return new Promise<boolean>((resolve) => {
    pendingNavigation?.(false);
    pendingNavigation = resolve;
    discardPromptOpen.value = true;
  });
});
onBeforeUnmount(() => {
  disposed = true;
  restoreRevision += 1;
  stateReadRevision += 1;
  settingsUpdateRevision += 1;
  pendingNavigation?.(false);
  for (const unlisten of unlisteners) unlisten();
});
useOutsidePointerDismiss(
  () => addMenuOpen.value || filterMenuOpen.value || actionsMenuOpen.value || !!openConfigMenuId.value,
  (target) => target instanceof Element
    && !!target.closest(".http-tts-top-add, .managed-list-toolbar__control, .managed-list-row__menu-wrap, .http-tts-actions-menu, .http-tts-filter-menu, .http-tts-config-menu"),
  () => {
  addMenuOpen.value = false;
  filterMenuOpen.value = false;
  actionsMenuOpen.value = false;
  openConfigMenuId.value = null;
  },
);
function keepEditor(): void {
  discardPromptOpen.value = false;
  pendingConfigId.value = null;
  pendingSelectionId.value = null;
  pendingCreate.value = false;
  const resume = pendingNavigation;
  pendingNavigation = null;
  resume?.(false);
}

function emptyFields(): HttpTtsConfigEditorFields {
  return {
    name: "",
    url: "",
    contentType: null,
    concurrentRate: "0",
    loginUrl: null,
    loginUi: null,
    loginCheckJs: null,
    header: null,
    jsLib: null,
    enabledCookieJar: null,
    enableDangerousApi: null,
  };
}

const actionsBlocked = computed(() => blocked.value || listBusy.value || importBusy.value || editBusy.value);

function toggleConfigMenu(configId: string, event: MouseEvent): void {
  filterMenuOpen.value = false;
  actionsMenuOpen.value = false;
  if (openConfigMenuId.value === configId) {
    openConfigMenuId.value = null;
    return;
  }
  const target = event.currentTarget;
  if (!(target instanceof HTMLElement)) return;
  const rect = target.getBoundingClientRect();
  const width = Math.max(1, Math.min(190, window.innerWidth - 16));
  const height = 82;
  const left = Math.max(8, Math.min(rect.right - width, window.innerWidth - width - 8));
  const below = rect.bottom + 5;
  const top = window.innerHeight - below >= height + 8
    ? below
    : Math.max(8, rect.top - height - 5);
  configMenuPosition.value = { top, left };
  openConfigMenuId.value = configId;
}

function toggleFilterMenu(event: MouseEvent): void {
  if (filterMenuOpen.value) {
    filterMenuOpen.value = false;
    return;
  }
  addMenuOpen.value = false;
  actionsMenuOpen.value = false;
  openConfigMenuId.value = null;
  const target = event.currentTarget;
  if (!(target instanceof HTMLElement)) return;
  const rect = target.getBoundingClientRect();
  const viewportWidth = document.documentElement.clientWidth || window.innerWidth;
  const width = 170;
  const height = 114;
  const left = Math.max(8, Math.min(rect.right - width, viewportWidth - width - 8));
  const below = rect.bottom + 5;
  const preferredTop = window.innerHeight - below >= height + 8 ? below : rect.top - height - 5;
  const top = Math.max(8, Math.min(preferredTop, window.innerHeight - height - 8));
  filterMenuPosition.value = { top, left };
  filterMenuOpen.value = true;
}

function toggleActionsMenu(event: MouseEvent): void {
  if (actionsMenuOpen.value) {
    actionsMenuOpen.value = false;
    return;
  }
  addMenuOpen.value = false;
  filterMenuOpen.value = false;
  openConfigMenuId.value = null;
  const target = event.currentTarget;
  if (!(target instanceof HTMLElement)) return;
  const rect = target.getBoundingClientRect();
  const viewportWidth = document.documentElement.clientWidth || window.innerWidth;
  const width = 180;
  const height = 82;
  const left = Math.max(8, Math.min(rect.right - width, viewportWidth - width - 8));
  const below = rect.bottom + 5;
  const preferredTop = window.innerHeight - below >= height + 8 ? below : rect.top - height - 5;
  const top = Math.max(8, Math.min(preferredTop, window.innerHeight - height - 8));
  actionsMenuPosition.value = { top, left };
  actionsMenuOpen.value = true;
}

function toggleBatchManagement(): void {
  batchMode.value = !batchMode.value;
  selectedConfigIds.value = [];
  actionsMenuOpen.value = false;
  openConfigMenuId.value = null;
}

function toggleConfigSelection(configId: string): void {
  if (actionsBlocked.value || deleteBusyId.value) return;
  const selected = new Set(selectedConfigIds.value);
  if (selected.has(configId)) selected.delete(configId);
  else selected.add(configId);
  selectedConfigIds.value = [...selected];
}

function selectVisibleConfigs(invert = false): void {
  if (actionsBlocked.value || deleteBusyId.value) return;
  const selected = new Set(selectedConfigIds.value);
  for (const config of filteredConfigs.value) {
    if (invert) {
      if (selected.has(config.id)) selected.delete(config.id);
      else selected.add(config.id);
    } else if (allVisibleSelected.value) {
      selected.delete(config.id);
    } else {
      selected.add(config.id);
    }
  }
  selectedConfigIds.value = [...selected];
}

function requestBulkExport(): void {
  if (actionsBlocked.value || deleteBusyId.value || !selectedConfigIds.value.length) return;
  httpTtsConfigActions.exportHttpTtsConfig(selectedConfigIds.value);
}

async function openEditor(configId: string = selectedConfigId.value): Promise<void> {
  if (!configId || actionsBlocked.value || deleteBusyId.value || editorLoading.value || blocked.value) return;
  openConfigMenuId.value = null;
  actionsMenuOpen.value = false;
  editorError.value = "";
  editorSuccess.value = "";
  if (editorOpen.value) {
    if (editingConfigId.value === configId) return;
    if (editorDirty.value) { pendingConfigId.value = configId; discardPromptOpen.value = true; return; }
    editorOpen.value = false;
  }
  editingConfigId.value = configId;
  editorOpen.value = true;
  editorLoading.value = true;
  try {
    const loaded = await httpTtsConfigActions.loadHttpTtsConfigEditorFields(configId);
    if (!loaded) {
      editorOpen.value = false;
      editorError.value = "读取朗读源配置失败，请重新选择引擎。";
      return;
    }
    Object.assign(fields, emptyFields(), loaded);
    initialEditorFields.value = JSON.stringify(fields);
  } catch (error) {
    editorOpen.value = false;
    editorError.value = error instanceof Error ? error.message : String(error);
  } finally {
    editorLoading.value = false;
  }
}

function openNewEditor(): void {
  if (actionsBlocked.value || deleteBusyId.value || editorLoading.value) return;
  if (editorOpen.value) {
    if (editorDirty.value) { pendingCreate.value = true; discardPromptOpen.value = true; return; }
    editorOpen.value = false;
  }
  editorError.value = "";
  editorSuccess.value = "";
  editingConfigId.value = "";
  Object.assign(fields, emptyFields());
  initialEditorFields.value = JSON.stringify(fields);
  editorOpen.value = true;
}

function addConfigAction(action: "new" | "file" | "url"): void {
  addMenuOpen.value = false;
  if (actionsBlocked.value || deleteBusyId.value) return;
  if (action === "new") openNewEditor();
  else if (action === "file") void importFromLocal();
  else openUrlImport();
}
function openUrlImport(): void {
  if (actionsBlocked.value || deleteBusyId.value) return;
  importUrl.value = "";
  importUrlError.value = "";
  importUrlOpen.value = true;
}
function closeUrlImport(): void {
  if (importUrlBusy.value || importBusy.value) return;
  importUrlOpen.value = false;
  importUrl.value = "";
  importUrlError.value = "";
}
async function restoreEditorFields(): Promise<void> {
  if (!editorOpen.value || actionsBlocked.value || deleteBusyId.value || editorLoading.value) return;
  editorError.value = "";
  editorSuccess.value = "";
  const id = editingConfigId.value;
  if (!id) {
    Object.assign(fields, emptyFields());
    initialEditorFields.value = JSON.stringify(fields);
    return;
  }
  editorLoading.value = true;
  try {
    const restored = await httpTtsConfigActions.loadHttpTtsConfigEditorFields(id);
    if (!restored) {
      editorError.value = "无法重新读取已保存配置，草稿已保留。";
      return;
    }
    if (!editorOpen.value || editingConfigId.value !== id) return;
    Object.assign(fields, emptyFields(), restored);
    initialEditorFields.value = JSON.stringify(fields);
    editorSuccess.value = "已从持久化配置重新读取。";
  } catch (error) {
    editorError.value = error instanceof Error ? error.message : String(error);
  } finally {
    editorLoading.value = false;
  }
}
function closeEditor(): void {
  if (editBusy.value || editorLoading.value) return;
  if (editorDirty.value) {
    discardPromptOpen.value = true;
    return;
  }
  editorOpen.value = false;
}
function requestDeleteConfig(configId: string): void {
  openConfigMenuId.value = null;
  if (actionsBlocked.value || deleteBusyId.value) return;
  if (editorDirty.value && editingConfigId.value === configId) {
    editorError.value = "请先保存或放弃当前配置修改，再删除此引擎。";
    return;
  }
  if (!configs.value.some(config => config.id === configId)) return;
  pendingDeleteIds.value = [configId];
}
function requestBulkDelete(): void {
  if (actionsBlocked.value || deleteBusyId.value || !selectedConfigIds.value.length) return;
  if (editorDirty.value && selectedConfigIds.value.includes(editingConfigId.value)) {
    editorError.value = "请先保存或放弃当前配置修改，再删除所选配置。";
    return;
  }
  pendingDeleteIds.value = [...selectedConfigIds.value];
}
function cancelDeleteConfig(): void {
  if (deleteBusyId.value) return;
  pendingDeleteIds.value = [];
}
function confirmDeleteConfig(): void {
  const ids = [...pendingDeleteIds.value];
  if (!ids.length || actionsBlocked.value || deleteBusyId.value) return;
  pendingDeleteIds.value = [];
  selectedConfigIds.value = selectedConfigIds.value.filter((id) => !ids.includes(id));
  httpTtsConfigActions.removeSelectedHttpTtsConfig(ids);
}

function selectPlaybackConfig(configId: string): void {
  openConfigMenuId.value = null;
  actionsMenuOpen.value = false;
  if (actionsBlocked.value || deleteBusyId.value || configId === selectedConfigId.value) return;
  if (editorDirty.value) {
    pendingSelectionId.value = configId;
    discardPromptOpen.value = true;
    return;
  }
  selectConfig(configId);
}
function discardEditor(): void {
  if (editBusy.value || editorLoading.value) return;
  const nextId = pendingConfigId.value;
  const selected = pendingSelectionId.value;
  const creating = pendingCreate.value;
  pendingConfigId.value = null;
  pendingSelectionId.value = null;
  pendingCreate.value = false;
  discardPromptOpen.value = false;
  editorOpen.value = false;
  const resume = pendingNavigation;
  pendingNavigation = null;
  if (resume) resume(true);
  else if (nextId) void openEditor(nextId);
  else if (creating) openNewEditor();
  else if (selected !== null) selectConfig(selected);
}

async function saveEditor(): Promise<void> {
  if (editorLoading.value || editBusy.value || blocked.value || !fields.name.trim() || !fields.url.trim()) return;
  if (editingConfigId.value && !editingConfig.value) {
    editorError.value = "当前引擎已被移除，不能从过期草稿保存。请返回列表重新选择。";
    return;
  }
  const initial = JSON.parse(initialEditorFields.value) as HttpTtsConfigEditorFields;
  const preserveUnchanged = (key: "contentType" | "concurrentRate" | "loginUrl" | "loginUi" | "loginCheckJs" | "header" | "jsLib"): string | null =>
    fields[key] === initial[key] ? fields[key] : cleanOptional(fields[key]);
  const editorFields: HttpTtsConfigEditorFields = {
    name: fields.name === initial.name ? fields.name : fields.name.trim(),
    url: fields.url === initial.url ? fields.url : fields.url.trim(),
    contentType: preserveUnchanged("contentType"),
    concurrentRate: preserveUnchanged("concurrentRate"),
    loginUrl: preserveUnchanged("loginUrl"),
    loginUi: preserveUnchanged("loginUi"),
    loginCheckJs: preserveUnchanged("loginCheckJs"),
    header: preserveUnchanged("header"),
    jsLib: preserveUnchanged("jsLib"),
    enabledCookieJar: fields.enabledCookieJar,
    enableDangerousApi: fields.enableDangerousApi,
  };
  const creating = !editingConfigId.value;
  editorError.value = "";
  editorSuccess.value = "";
  try {
    const saved = await httpTtsConfigActions.saveHttpTtsConfigFromEditor(creating ? null : editingConfigId.value, editorFields);
    if (!saved) {
      editorError.value = "保存没有完成。当前修改仍在编辑器中，请检查配置后重试。";
      return;
    }
    initialEditorFields.value = JSON.stringify(fields);
    if (creating || window.matchMedia("(max-width: 860px)").matches) {
      // A successful create must not leave the editor in "new" mode: the next save would create a duplicate.
      editorOpen.value = false;
    } else {
      editorSuccess.value = "配置已保存。";
    }
  } catch (error) {
    editorError.value = error instanceof Error ? error.message : String(error);
  }
}

function cleanOptional(value: string | null): string | null {
  const valueText = value ?? "";
  return valueText.trim() ? valueText : null;
}

async function importFromLocal(): Promise<void> {
  if (actionsBlocked.value || deleteBusyId.value) return;
  await httpTtsConfigActions.importHttpTtsConfigFromLocal();
}

async function importFromLink(): Promise<void> {
  if (!importUrl.value.trim() || actionsBlocked.value || deleteBusyId.value) return;
  importUrlError.value = "";
  importUrlBusy.value = true;
  try {
    const imported = await httpTtsConfigActions.importHttpTtsConfigFromLink(importUrl.value);
    if (imported) {
      importUrlOpen.value = false;
      importUrl.value = "";
    } else {
      importUrlError.value = error.value || "导入没有完成，请检查链接后重试。";
    }
  } catch (error) {
    importUrlError.value = error instanceof Error ? error.message : String(error);
  } finally {
    importUrlBusy.value = false;
  }
}
function openInitialDesktopEditor(): void {
  if (!isMounted.value || initialDesktopOpened.value || editorOpen.value || listBusy.value || actionsBlocked.value || !configs.value.length || !window.matchMedia("(min-width: 861px)").matches) return;
  initialDesktopOpened.value = true;
  const candidate = configs.value.find((config) => config.id === selectedConfigId.value) ?? configs.value[0];
  void openEditor(candidate.id);
}
onMounted(async () => {
  isMounted.value = true;
  const unlistenSettings = await listen<ResourceDescriptor>("settings-updated", (event) => {
    void loadSettingsDescriptor(event.payload);
  });
  if (disposed) { unlistenSettings(); return; }
  unlisteners.push(unlistenSettings);
  const unlistenResources = await listen<{ kind: string; resource: ResourceDescriptor }>("resource-updated", (event) => {
    handleResourceUpdated(event.payload);
  });
  if (disposed) { unlistenResources(); return; }
  unlisteners.push(unlistenResources);
  const unlistenRestore = await listen<AppBootstrap>("app-state-updated", (event) => {
    handleRestore(event.payload);
  });
  if (disposed) { unlistenRestore(); return; }
  unlisteners.push(unlistenRestore);
  await loadPageData();
  openInitialDesktopEditor();
});
watch(() => configs.value.map(config => config.id), ids => {
  // A successful removal refreshes the authoritative Rust list.
  if (editingConfigId.value && !ids.includes(editingConfigId.value) && !editorDirty.value) {
    editorOpen.value = false;
    editingConfigId.value = "";
  }
  const existingIds = new Set(ids);
  selectedConfigIds.value = selectedConfigIds.value.filter((id) => existingIds.has(id));
  pendingDeleteIds.value = pendingDeleteIds.value.filter((id) => existingIds.has(id));
});
watch(() => [listBusy.value, configs.value.length, selectedConfigId.value, blocked.value], openInitialDesktopEditor);
</script>

<template>
  <section class="settings-card http-tts-config-card" :class="{ 'http-tts-config-card--empty': !editorOpen && configs.length === 0 }">
    <header class="http-tts-topbar">
      <BackButton label="返回高级设置" @click="returnToAdvancedSettings" />
      <div class="http-tts-top-title"><strong>TTS 引擎</strong></div>
      <div class="http-tts-top-add">
        <button type="button" class="button primary"
:disabled="actionsBlocked || !!deleteBusyId"
          @click="addMenuOpen = !addMenuOpen; filterMenuOpen = false; actionsMenuOpen = false; openConfigMenuId = null"><PrototypeIcon name="plus"/> 添加</button>
        <div v-if="addMenuOpen" class="http-tts-add-menu">
          <button type="button" @click="addConfigAction('new')">新建配置</button>
          <button type="button" @click="addConfigAction('file')">本地 JSON 导入</button>
          <button type="button" @click="addConfigAction('url')">网络链接导入</button>
        </div>
      </div>
    </header>
    <div class="http-tts-overview" :class="{ 'editor-active': editorOpen }">
    <nav class="http-tts-master-list">
      <div class="http-tts-master-summary">
        <span class="http-tts-count"><strong>{{ configs.filter(config => config.enabled).length }} 个启用</strong><small>{{ configs.length }} 个配置</small></span>
      </div>
      <ManagedListToolbar
        v-model:query="configQuery"
        search-placeholder="搜索 TTS 配置"
        :filter-active="enabledFilter !== 'all'"
        :filter-expanded="filterMenuOpen"
        :actions-expanded="actionsMenuOpen"
        @toggle-filter="toggleFilterMenu"
        @toggle-actions="toggleActionsMenu"
      >
        <template #actions-menu>
          <Teleport to="body">
            <div v-if="actionsMenuOpen" class="http-tts-actions-menu" :style="{ top: actionsMenuPosition.top + 'px', left: actionsMenuPosition.left + 'px' }">
              <button type="button" :disabled="listBusy || importBusy || editBusy || blocked || settingsLoading || !!deleteBusyId"
                @click="actionsMenuOpen = false; refreshPageData()">刷新配置</button>
              <button type="button" @click="toggleBatchManagement">{{ batchMode ? '退出批量管理' : '批量管理' }}</button>
            </div>
          </Teleport>
        </template>
        <template #filter-menu>
          <Teleport to="body">
            <div v-if="filterMenuOpen" class="http-tts-filter-menu" :style="{ top: filterMenuPosition.top + 'px', left: filterMenuPosition.left + 'px' }">
              <button type="button" @click="enabledFilter = 'all'; filterMenuOpen = false">全部配置</button>
              <button type="button" @click="enabledFilter = 'enabled'; filterMenuOpen = false">仅启用</button>
              <button type="button" @click="enabledFilter = 'disabled'; filterMenuOpen = false">仅停用</button>
            </div>
          </Teleport>
        </template>
      </ManagedListToolbar>
      <div class="http-tts-items-scroll" @scroll.passive="openConfigMenuId = null; filterMenuOpen = false; actionsMenuOpen = false">
      <ManagedListRow name="系统朗读" subtitle="不使用远程朗读" :enabled="false"
        :active="selectedConfigId === ''" :busy="listBusy || editBusy || blocked" :show-menu="false"
        @open="selectPlaybackConfig('')" />
      <ManagedListRow v-for="config in filteredConfigs" :key="config.id"
        :name="config.name" :subtitle="config.enabled ? 'HTTP TTS' : '已停用'" :enabled="config.enabled"
        :active="selectedConfigId === config.id || (batchMode && selectedConfigIds.includes(config.id))" :busy="actionsBlocked || !!deleteBusyId"
        :batch-mode="batchMode" :selected="selectedConfigIds.includes(config.id)"
        :show-menu="!batchMode"
        @open="batchMode ? toggleConfigSelection(config.id) : openEditor(config.id)"
        @select="toggleConfigSelection(config.id)"
      >
        <template #menu-trigger>
          <button type="button" class="http-tts-config-menu-button" :class="{ 'is-open': openConfigMenuId === config.id }"
            :disabled="actionsBlocked || !!deleteBusyId"
            @click.stop="toggleConfigMenu(config.id, $event)"><PrototypeIcon name="more"/></button>
        </template>
      </ManagedListRow>
      <p v-if="!filteredConfigs.length" class="http-tts-list-empty">{{ configs.length ? '没有符合条件的 TTS 配置' : '还没有 TTS 配置' }}</p>
      </div>
      <footer v-if="batchMode" class="http-tts-selection-bar">
        <button type="button" class="http-tts-selection-text-button" :disabled="actionsBlocked || !!deleteBusyId || !filteredConfigs.length"
          @click="selectVisibleConfigs()">{{ allVisibleSelected ? '取消全选' : '全选' }}</button>
        <button type="button" class="http-tts-selection-text-button" :disabled="actionsBlocked || !!deleteBusyId || !filteredConfigs.length"
          @click="selectVisibleConfigs(true)">反选</button>
        <span class="http-tts-selection-count">已选 {{ selectedConfigs.length }}</span>
        <button type="button" class="http-tts-selection-text-button" :disabled="actionsBlocked || !!deleteBusyId || !selectedConfigs.length"
          @click="requestBulkExport">导出</button>
        <button type="button" class="http-tts-selection-text-button http-tts-selection-delete" :disabled="actionsBlocked || !!deleteBusyId || !selectedConfigs.length"
          @click="requestBulkDelete">删除</button>
      </footer>
    </nav>
    <Teleport to="body">
      <div v-if="openConfigMenu" class="http-tts-config-menu" :style="{ top: configMenuPosition.top + 'px', left: configMenuPosition.left + 'px' }">
        <button type="button" :disabled="actionsBlocked || !!deleteBusyId || !openConfigMenu.enabled || selectedConfigId === openConfigMenu.id" @click="selectPlaybackConfig(openConfigMenu.id)">设为当前朗读</button>
        <button type="button" :disabled="actionsBlocked || !!deleteBusyId" @click="requestDeleteConfig(openConfigMenu.id)">删除配置</button>
      </div>
    </Teleport>
    <p v-if="!importUrlOpen && (error || (!editorOpen && editorError))" class="source-identity-confirmation">{{ editorError || error }}</p>

    </div>
    <aside v-if="!editorOpen && selectedConfig" class="http-tts-preview">
      <span class="http-tts-preview-icon"><PrototypeIcon name="tts"/></span>
      <strong>{{ selectedConfig.name }}</strong>
      <p>{{ selectedConfig.enabled ? '当前选择的网络朗读源' : '当前配置已停用' }}。参数由 Rust 保存，选择编辑后可以修改原始配置字段。</p>
      <button type="button" class="button primary small" :disabled="actionsBlocked || !!deleteBusyId" @click="openEditor()">编辑配置</button>
    </aside>
    <div v-if="importUrlOpen" class="http-tts-import-scrim"
      @click.self="closeUrlImport">
      <section class="http-tts-import-dialog">
        <h3 id="http-tts-import-title">从网络链接导入</h3>
        <form class="http-tts-import-url-form" @submit.prevent="importFromLink">
          <label for="http-tts-config-url">HTTP TTS 配置 JSON 链接</label>
          <input id="http-tts-config-url" v-model="importUrl" type="url" required
            inputmode="url" autocomplete="url" maxlength="4096"
            placeholder="https://example.com/http-tts.json" :disabled="importUrlBusy || actionsBlocked || !!deleteBusyId" />
          <button type="submit" class="button primary"
            :disabled="importUrlBusy || actionsBlocked || !!deleteBusyId || !importUrl.trim()">
            {{ importUrlBusy || importBusy ? '下载并导入中…' : '下载并导入' }}
          </button>
          <small>下载完成后会直接导入配置，上限 1 MiB。</small>
        </form>
        <p v-if="importUrlError" class="http-tts-import-error">{{ importUrlError }}</p>
        <div class="http-tts-import-actions">
          <button type="button" class="button secondary" :disabled="importUrlBusy || importBusy" @click="closeUrlImport">取消</button>
        </div>
      </section>
    </div>
    <section v-if="editorOpen" class="http-tts-editor-page">
      <article class="http-tts-editor-dialog">
        <header class="http-tts-editor-header">
          <button type="button" class="http-tts-editor-close" :disabled="editorLoading || editBusy" @click="closeEditor"><PrototypeIcon name="left"/></button>
          <div>
            <h2 id="http-tts-editor-title">{{ creatingConfig ? '新建 HTTP TTS 配置' : editingConfig?.name ?? '编辑朗读源' }}</h2>
          </div>
          <div class="http-tts-editor-header-actions">
            <button v-if="editingConfig" type="button" class="button danger" :disabled="actionsBlocked || !!deleteBusyId" @click="requestDeleteConfig(editingConfig.id)">删除</button>
            <button type="button" class="button primary" :disabled="blocked || editorLoading || editBusy || !fields.name.trim() || !fields.url.trim()" @click="saveEditor">{{ editBusy ? '保存中…' : '保存' }}</button>
          </div>
        </header>

        <div class="http-tts-editor-body">
          <p v-if="editorLoading" class="http-tts-editor-loading">正在读取配置…</p>
          <template v-else>
            <fieldset class="http-tts-editor-fields" :disabled="blocked || editBusy">
              <section class="http-tts-editor-section"><header><strong>请求</strong></header>
              <div class="http-tts-editor-grid">
            <label class="form-field">
              <span>名称</span>
              <input v-model="fields.name" maxlength="256" autocomplete="off" placeholder="朗读源名称" />
            </label>
            <label class="form-field">
              <span>URL</span>
              <input v-model="fields.url" type="url" inputmode="url" autocomplete="url" placeholder="https://example.com/tts" />
            </label>
            <label class="form-field">
              <span>Content-Type</span>
              <input v-model="fields.contentType" autocomplete="off" placeholder="application/json" />
            </label>
            <label class="form-field">
              <span>并发率（concurrentRate）</span>
              <input v-model="fields.concurrentRate" inputmode="numeric" autocomplete="off" placeholder="0" />
            </label>
              </div>
              <label class="form-field">
                <span>请求头（header）</span>
                <textarea v-model="fields.header" rows="4" spellcheck="false" placeholder="可选的请求头配置"></textarea>
                <small>对象／数组格式请填写有效 JSON，保存后保持原始数据类型；原本是字符串的来源仍按字符串保存。</small>
              </label>
              </section>
              <section class="http-tts-editor-section"><header><strong>登录与扩展</strong></header>
            <label class="form-field">
              <span>登录 URL（loginUrl）</span>
              <input v-model="fields.loginUrl" type="url" inputmode="url" autocomplete="url" placeholder="可选" />
            </label>
            <label class="form-field">
              <span>登录 UI（loginUi）</span>
              <textarea v-model="fields.loginUi" rows="5" spellcheck="false" placeholder="可选的登录界面配置"></textarea>
              <small>现有 JSON 数组／对象不会被转为字符串；如果原值是字符串，编辑后仍保存字符串。</small>
            </label>
            <label class="form-field">
              <span>登录检查 JS（loginCheckJs）</span>
              <textarea v-model="fields.loginCheckJs" rows="5" spellcheck="false" placeholder="可选"></textarea>
            </label>
              </section>
              <section class="http-tts-editor-section"><header><strong>脚本与权限</strong></header>
                <label class="form-field">
                  <span>共享 JS 库（jsLib）</span>
                  <textarea v-model="fields.jsLib" rows="6" spellcheck="false" placeholder="可选的 JavaScript 辅助函数"></textarea>
                </label>
                <label class="http-tts-editor-toggle">
                  <input v-model="fields.enabledCookieJar" type="checkbox" :true-value="true" :false-value="false"/>
                  <span><strong>启用 Cookie Jar</strong><small>允许该引擎在请求间使用 Cookie 存储</small></span>
                </label>
                <label class="http-tts-editor-toggle">
                  <input v-model="fields.enableDangerousApi" type="checkbox" :true-value="true" :false-value="false"/>
                  <span><strong>允许危险 JS API</strong><small>仅对了解并信任来源脚本的配置开启</small></span>
                </label>
              </section>
            </fieldset>
            <div class="http-tts-editor-bottom">
              <button v-if="editingConfig" type="button" class="button danger http-tts-editor-bottom-delete" :disabled="actionsBlocked || !!deleteBusyId" @click="requestDeleteConfig(editingConfig.id)">删除配置</button>
              <button type="button" class="button secondary"
                :disabled="blocked || editorLoading || editBusy || !editorDirty"
                @click="restoreEditorFields">还原</button>
              <button type="button" class="button primary" :disabled="blocked || editorLoading || editBusy || !fields.name.trim() || !fields.url.trim()" @click="saveEditor">{{ editBusy ? '保存中…' : '保存' }}</button>
            </div>
            <p v-if="editorError || error" class="source-identity-confirmation">{{ editorError || error }}</p>
            <p v-if="editorSuccess" class="http-tts-editor-success">{{ editorSuccess }}</p>
          </template>
        </div>
      </article>
    </section>
    <div v-if="pendingDeleteIds.length" class="http-tts-discard-mask"
      @click.self="cancelDeleteConfig">
      <section class="http-tts-discard-dialog">
        <h3 id="tts-delete-title">删除{{ pendingDeleteIds.length === 1 ? ' HTTP TTS 配置？' : ` ${pendingDeleteIds.length} 个 HTTP TTS 配置？` }}</h3>
        <p>{{ pendingDeleteIds.length === 1 ? `“${pendingDeleteConfigs[0]?.name ?? '此引擎'}”将从应用中删除。` : `选中的 ${pendingDeleteIds.length} 个配置将从应用中删除。` }}此操作不能撤销，删除当前朗读源后会切回系统朗读。</p>
        <div><button type="button" class="button secondary" @click="cancelDeleteConfig">取消</button>
          <button type="button" class="button danger" :disabled="actionsBlocked || !!deleteBusyId" @click="confirmDeleteConfig">确认删除</button></div>
      </section>
    </div>
    <div v-if="discardPromptOpen" class="http-tts-discard-mask" @click.self="keepEditor">
      <section class="http-tts-discard-dialog">
        <h3 id="tts-discard-title">放弃未保存的修改？</h3>
        <p>当前朗读引擎的字段尚未保存。返回、切换配置或更换当前播放引擎都会丢弃未保存的修改。</p>
        <div><button type="button" class="button secondary" @click="keepEditor">继续编辑</button>
          <button type="button" class="button danger" @click="discardEditor">放弃修改</button></div>
      </section>
    </div>
  </section>
</template>

<style scoped>
.http-tts-config-card {
  --tts-line: var(--app-line);
  --tts-muted: var(--app-muted);
  --tts-accent: var(--app-accent);
  display: grid;
  grid-template-columns: 300px minmax(0, 1fr);
  grid-template-rows: 64px minmax(0, 1fr);
  gap: 0;
  width: 100%;
  min-width: 0;
  height: 100%;
  min-height: 0;
  padding: 0;
  border: 0;
  border-radius: 0;
  background: var(--app-panel-softer);
  color: var(--app-text);
}
.http-tts-config-card--empty{grid-template-columns:minmax(0,1fr)}
.http-tts-topbar{position:sticky;top:0;z-index:35;box-sizing:border-box;grid-column:1/-1;grid-row:1;display:grid;grid-template-columns:44px minmax(0,1fr) auto;align-items:center;gap:10px;min-height:64px;padding:0 max(18px,calc((100% - 1180px)/2));border-bottom:1px solid var(--tts-line);background:#f6f7f9ef;backdrop-filter:blur(18px)}
.http-tts-top-title{min-width:0}
.http-tts-top-title strong{font-size:14px;color:#242631;font-weight:750}
.http-tts-top-add{position:relative}
.http-tts-top-add>.button{display:inline-flex;align-items:center;gap:6px;min-height:34px;padding:0 12px;font-size:10px;background:var(--tts-accent);color:#fff}
.http-tts-top-add>.button :deep(svg){width:15px;height:15px}
.http-tts-add-menu{position:absolute;right:0;top:calc(100% + 7px);z-index:50;display:grid;gap:3px;width:232px;padding:7px;border:1px solid var(--tts-line);border-radius:11px;background:#fff;box-shadow:0 16px 40px #20203322}
.http-tts-add-menu button{display:flex;align-items:center;min-height:38px;padding:9px;border:0;border-radius:8px;background:transparent;color:#343640;text-align:left;font-size:11px;font-weight:650;cursor:pointer}
.http-tts-add-menu button:hover{background:var(--app-accent-soft)}
.http-tts-overview{box-sizing:border-box;grid-column:1;grid-row:2;min-width:0;height:100%;min-height:0;display:flex;flex-direction:column;overflow:hidden;padding:14px;border-right:1px solid var(--tts-line);background:#fafafd}
.http-tts-master-list{display:flex;flex:1 1 auto;flex-direction:column;gap:3px;min-height:0;margin:0;overflow:hidden}
.http-tts-items-scroll{display:grid;flex:1 1 auto;align-content:start;gap:3px;min-height:0;margin-right:-14px;padding-right:14px;overflow-y:auto;overscroll-behavior:contain}
.http-tts-master-summary{display:flex;align-items:center;gap:6px;padding:3px}
.http-tts-count{display:flex;flex:1;align-items:center;justify-content:space-between;gap:9px}
.http-tts-count strong,.http-tts-count small{font-size:9px;font-weight:500;color:var(--tts-muted)}
.http-tts-filter-menu{position:fixed;z-index:1000;box-sizing:border-box;display:grid;width:min(170px,calc(100vw - 16px));max-height:calc(100vh - 16px);overflow-y:auto;padding:5px;border:1px solid var(--tts-line,var(--app-line));border-radius:11px;background:#fff;box-shadow:0 12px 28px #22223322}
.http-tts-filter-menu button{width:100%;min-height:34px;padding:0 10px;border:0;border-radius:7px;background:transparent;color:#343640;text-align:left;font:inherit;font-size:12px;cursor:pointer}
.http-tts-filter-menu button:hover{background:#f1eefd}
.http-tts-actions-menu{position:fixed;z-index:1000;box-sizing:border-box;display:grid;width:min(180px,calc(100vw - 16px));max-height:calc(100vh - 16px);overflow-y:auto;padding:5px;border:1px solid var(--tts-line,var(--app-line));border-radius:11px;background:#fff;box-shadow:0 12px 28px #22223322}
.http-tts-actions-menu button{width:100%;min-height:34px;padding:0 10px;border:0;border-radius:7px;background:transparent;color:#343640;text-align:left;font:inherit;font-size:12px;cursor:pointer}
.http-tts-actions-menu button:hover:not(:disabled){background:#f1eefd}
.http-tts-actions-menu button:disabled{opacity:.4;cursor:default}
.http-tts-selection-bar{display:flex;flex:none;flex-wrap:wrap;align-items:center;gap:5px;margin-top:auto;padding:8px 3px;border-top:1px solid var(--tts-line);background:#fafafd}
.http-tts-selection-text-button{min-height:30px;padding:0 7px;border:0;border-radius:7px;background:transparent;color:#686d78;font:inherit;font-size:10px;cursor:pointer}
.http-tts-selection-text-button:hover:not(:disabled){background:var(--app-accent-soft);color:var(--app-accent-ink)}
.http-tts-selection-text-button:disabled{opacity:.45;cursor:default}
.http-tts-selection-count{margin-left:auto;color:var(--tts-muted);font-size:10px}
.http-tts-selection-delete{color:#b7545e}
.http-tts-config-menu-button{display:grid;place-items:center;width:30px;height:34px;padding:0;border:0;border-radius:8px;background:transparent;color:#737783;cursor:pointer}
.http-tts-config-menu-button:hover:not(:disabled),.http-tts-config-menu-button.is-open{background:var(--app-accent-soft);color:var(--app-accent-ink)}
.http-tts-config-menu-button:disabled{opacity:.45;cursor:default}
.http-tts-config-menu-button :deep(svg){width:17px;height:17px}
.http-tts-config-menu{position:fixed;z-index:1000;box-sizing:border-box;width:min(190px,calc(100vw - 16px));max-height:calc(100vh - 16px);overflow-y:auto;padding:5px;border:1px solid var(--tts-line,var(--app-line));border-radius:11px;background:#fff;box-shadow:0 12px 28px #22223322}
.http-tts-config-menu button{width:100%;min-height:34px;padding:0 10px;border:0;border-radius:7px;background:transparent;color:#343640;text-align:left;font:inherit;font-size:12px;cursor:pointer}
.http-tts-config-menu button:hover:not(:disabled){background:#f1eefd}
.http-tts-config-menu button:disabled{opacity:.4;cursor:default}
.http-tts-list-empty{margin:0;padding:24px 10px;color:var(--tts-muted);font-size:12px;text-align:center}
.http-tts-import-scrim{position:fixed;z-index:2000;inset:0;display:grid;place-items:center;padding:16px;background:#191b2780}
.http-tts-import-dialog{box-sizing:border-box;width:min(420px,100%);max-height:min(82dvh,700px);overflow:auto;padding:22px;border:1px solid var(--tts-line);border-radius:14px;background:#fff;box-shadow:0 20px 40px #11112233}
.http-tts-import-dialog h3{margin:0;font-size:17px}
.http-tts-import-url-form{display:grid;gap:9px;margin:10px 0 14px}
.http-tts-import-url-form label{font-size:12px;font-weight:650}
.http-tts-import-url-form input{box-sizing:border-box;width:100%;min-height:39px;padding:9px 11px;border:1px solid var(--tts-line);border-radius:9px;background:#fff;color:#343640;font:inherit;font-size:12px}
.http-tts-import-url-form button{justify-self:start}
.http-tts-import-url-form small{color:var(--tts-muted);font-size:11px;line-height:1.5}
.http-tts-import-error{margin:9px 0 18px;color:#b84955;font-size:12px;line-height:1.5}
.http-tts-import-actions{display:flex;justify-content:flex-end;gap:8px}
.http-tts-overview>.source-identity-confirmation{margin-top:10px}
.http-tts-preview{grid-column:2;grid-row:2;display:grid;align-content:center;justify-items:center;gap:12px;min-height:300px;padding:20px;background:#fafafd;color:var(--tts-muted);text-align:center}
.http-tts-preview-icon{display:grid;place-items:center;width:44px;height:44px;border-radius:12px;background:var(--app-accent-soft);color:var(--tts-accent)}
.http-tts-preview-icon :deep(svg){width:24px;height:24px}
.http-tts-preview strong{font-size:13px;color:#353743}
.http-tts-preview p{max-width:290px;margin:0;color:#9295a1;font-size:11px;line-height:1.65}
.http-tts-editor-page{grid-column:2;grid-row:2;min-width:0;min-height:0;overflow-y:auto;background:#fff}
.http-tts-editor-dialog{width:100%;background:#fff}
.http-tts-editor-header{display:grid;grid-template-columns:36px minmax(0,1fr) auto;align-items:center;gap:12px;padding:28px 34px 16px}
.http-tts-editor-header h2{margin:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:#272934;font-size:17px}
.http-tts-editor-close{display:grid;place-items:center;width:36px;height:36px;border:0;border-radius:9px;background:transparent;color:#747983;cursor:pointer}
.http-tts-editor-close :deep(svg){width:19px;height:19px}
.http-tts-editor-header-actions{display:flex;align-items:center;gap:8px}
.http-tts-editor-header-actions>.button{display:none}
.http-tts-editor-body{padding:0 34px 48px}
.http-tts-editor-loading{padding:28px 0;color:var(--tts-muted);font-size:11px;text-align:center}
.http-tts-editor-fields{display:block;min-width:0;margin:0;padding:0;border:0}
.http-tts-editor-section{padding:18px 0;border-top:1px solid var(--tts-line)}
.http-tts-editor-section>header{display:grid;gap:3px;margin-bottom:12px}
.http-tts-editor-section>header strong{font-size:11px;color:#343640}
.http-tts-editor-grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:0 12px}
.http-tts-editor-grid .form-field:nth-child(2){grid-column:1/-1}
.http-tts-editor-body .form-field{display:grid;gap:6px;margin:0 0 14px;min-width:0}
.http-tts-editor-body .form-field>span{font-size:9px;font-weight:600;color:#777c85}
.http-tts-editor-body .form-field>small{font-size:8px;line-height:1.6;color:var(--tts-muted)}
.http-tts-editor-body input:not([type=checkbox]),.http-tts-editor-body textarea{box-sizing:border-box;width:100%;min-width:0;min-height:36px;padding:8px 9px;border:1px solid var(--tts-line);border-radius:9px;background:#fff;color:#343640;font-size:10px;line-height:1.5}
.http-tts-editor-body textarea{min-height:88px;resize:vertical;font-family:ui-monospace,SFMono-Regular,Consolas,monospace}
.http-tts-editor-toggle{display:flex;align-items:flex-start;gap:11px;padding:11px 0;border-bottom:1px solid #eceef2;cursor:pointer}
.http-tts-editor-toggle input{flex:none;width:17px;height:17px;margin:2px 0 0;accent-color:var(--tts-accent)}
.http-tts-editor-toggle>span{display:grid;gap:4px;min-width:0}
.http-tts-editor-toggle strong{font-size:10px;color:#3c3e4a}
.http-tts-editor-toggle small{font-size:8px;color:var(--tts-muted);line-height:1.5}
.http-tts-editor-bottom{display:flex;align-items:center;justify-content:flex-end;gap:8px;padding-top:17px;border-top:1px solid var(--tts-line)}
.http-tts-editor-bottom-delete{margin-right:auto}
.http-tts-editor-bottom .button{min-height:34px;font-size:10px}
.http-tts-editor-bottom .button.primary{background:var(--tts-accent)}
.http-tts-editor-header-actions .button.danger,.http-tts-editor-bottom .button.danger{background:#fff0f2;color:#af525d}
.http-tts-editor-success{margin:10px 0;padding:10px 12px;border-radius:9px;background:#f0f8f1;color:#497555;font-size:11px}
.http-tts-discard-mask{position:fixed;z-index:100;inset:0;display:grid;place-items:center;padding:16px;background:#191b2b88}
.http-tts-discard-dialog{box-sizing:border-box;width:min(410px,100%);padding:22px;border:1px solid var(--tts-line);border-radius:14px;background:#fff;box-shadow:0 18px 45px #14162828}
.http-tts-discard-dialog h3{margin:0 0 11px;color:#373845;font-size:15px}
.http-tts-discard-dialog p{margin:0;color:#777d8a;font-size:11px;line-height:1.7}
.http-tts-discard-dialog>div{display:flex;justify-content:flex-end;gap:8px;margin-top:19px}
@media(max-width:860px){
 .http-tts-config-card{display:flex;flex-direction:column;height:100%;min-height:0}
 .http-tts-topbar{padding:0 14px}
 .http-tts-overview{flex:1 1 auto;height:auto;min-height:0;max-height:none;padding:12px 14px;border:0;background:#fafafd}
 .http-tts-preview{display:none}
 .http-tts-config-card:has(.http-tts-editor-page) .http-tts-topbar,
 .http-tts-config-card:has(.http-tts-editor-page) .http-tts-overview{display:none}
 .http-tts-editor-header{position:sticky;top:0;z-index:30;grid-template-columns:36px minmax(0,1fr) auto;gap:8px;box-sizing:border-box;min-height:64px;padding:0 14px;border-bottom:1px solid var(--tts-line);background:#f6f7f9ef;backdrop-filter:blur(14px)}
 .http-tts-editor-close{display:grid}
 .http-tts-editor-header h2{font-size:12px}
 .http-tts-editor-header-actions>.button{display:inline-flex;align-items:center;min-height:34px;padding:0 9px;font-size:10px}
 .http-tts-editor-header-actions>.button.danger{display:none}
 .http-tts-editor-body{padding:16px 14px 54px}
 .http-tts-editor-page{flex:1 1 auto;min-height:0;overflow-y:auto}
 .http-tts-editor-grid{grid-template-columns:1fr}
 .http-tts-editor-grid .form-field:nth-child(2){grid-column:auto}
 .http-tts-editor-bottom{display:none}
}
@media(max-width:400px){
 .http-tts-topbar{padding:0 12px}
 .http-tts-editor-header{gap:5px;padding:0 10px}
 .http-tts-editor-header-actions>.button{padding:0 7px}
}
</style>

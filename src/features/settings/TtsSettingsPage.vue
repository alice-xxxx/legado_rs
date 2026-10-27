<script setup lang="ts">
import { Dialog as KDialog, DialogButton as KDialogButton, Popover as KPopover, Toggle as KToggle, Glass as KGlass, Button as KButton } from "konsta/vue";
import SubpageHeader from "../../ui/SubpageHeader.vue";
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { onBeforeRouteLeave } from "vue-router";
import { httpTtsConfigs } from "./configurationStores";
import type { AppBootstrap, HttpTtsConfigEditorFields } from "../../api/types";
import { useHttpTtsConfigActions } from "./useHttpTtsConfigActions";
import { settings } from "./settingsStore";
import ManagedListRow from "../../ui/ManagedListRow.vue";
import ManagedListToolbar from "../../ui/ManagedListToolbar.vue";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import { useOutsidePointerDismiss } from "../../ui/useOutsidePointerDismiss";
import { routeNames } from "../../router";
import { returnToRoute } from "../../app/routeReturn";
import { backupBusy } from "./backupState";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { notify } from "../../app/notifications";
import { selectReaderHttpTtsConfig } from "../reader/readerTtsConfigService";

const props = defineProps<{ embedded?: boolean }>();
const emit = defineEmits<{ back: [] }>();
const configs = httpTtsConfigs;
const importBusy = ref(false);
const error = ref("");
const blocked = computed(() => backupBusy.value !== null || shelfBatchRecoveryRequired.value);
const selectedConfigId = computed(() => settings.value.reader.httpTtsConfigId ?? "");
let disposed = false;
let restoreRevision = 0;
const unlisteners: UnlistenFn[] = [];

function errorText(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason);
}

function selectConfig(id: string): void {
  selectReaderHttpTtsConfig(id);
}

const httpTtsConfigActions = useHttpTtsConfigActions({
  error,
  importBusy,
  selectConfig,
  getRestoreRevision: () => restoreRevision,
  notify,
  errorText,
});

function handleRestore(): void {
  restoreRevision++;
  httpTtsConfigActions.resetForRestore();
  editorOpen.value = false;
  editingConfigId.value = "";
  selectedConfigIds.value = [];
  pendingDeleteIds.value = [];
}

function returnToAdvancedSettings(): void {
  if (props.embedded) {
    if (editorOpen.value) closeEditor();
    else emit("back");
  } else returnToRoute(routeNames.settingsOther);
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
const addMenuTarget = ref<HTMLElement | null>(null);
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
const editingConfigId = ref("");
const fields = reactive<HttpTtsConfigEditorFields>(emptyFields());
const editorDirty = computed(() => editorOpen.value && JSON.stringify(fields) !== initialEditorFields.value);
let pendingNavigation: ((allow: boolean) => void) | null = null;
if (!props.embedded) onBeforeRouteLeave(() => {
  if (!editorOpen.value) return true;
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
  pendingNavigation?.(false);
  for (const unlisten of unlisteners) unlisten();
});
useOutsidePointerDismiss(
  () => addMenuOpen.value || filterMenuOpen.value || actionsMenuOpen.value || !!openConfigMenuId.value,
  (target) => target instanceof Element
    && !!target.closest(".http-tts-add-menu, .http-tts-top-add, .managed-list-toolbar__control, .managed-list-row__menu-wrap, .http-tts-actions-menu, .http-tts-filter-menu, .http-tts-config-menu"),
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

const actionsBlocked = computed(() => blocked.value || importBusy.value);

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
  if (actionsBlocked.value) return;
  const selected = new Set(selectedConfigIds.value);
  if (selected.has(configId)) selected.delete(configId);
  else selected.add(configId);
  selectedConfigIds.value = [...selected];
}

function selectVisibleConfigs(invert = false): void {
  if (actionsBlocked.value) return;
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
  if (actionsBlocked.value || !selectedConfigIds.value.length) return;
  httpTtsConfigActions.exportHttpTtsConfig(selectedConfigIds.value);
}

function openEditor(configId: string = selectedConfigId.value): void {
  if (!configId || actionsBlocked.value || blocked.value) return;
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
  try {
    const loaded = httpTtsConfigActions.loadHttpTtsConfigEditorFields(configId);
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
  }
}

function openNewEditor(): void {
  if (actionsBlocked.value) return;
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
  if (actionsBlocked.value) return;
  if (action === "new") openNewEditor();
  else if (action === "file") void importFromLocal();
  else openUrlImport();
}
function openUrlImport(): void {
  if (actionsBlocked.value) return;
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
function restoreEditorFields(): void {
  if (!editorOpen.value || actionsBlocked.value) return;
  editorError.value = "";
  editorSuccess.value = "";
  const id = editingConfigId.value;
  if (!id) {
    Object.assign(fields, emptyFields());
    initialEditorFields.value = JSON.stringify(fields);
    return;
  }
  try {
    const restored = httpTtsConfigActions.loadHttpTtsConfigEditorFields(id);
    if (!restored) {
      editorError.value = "无法重新读取已保存配置，草稿已保留。";
      return;
    }
    if (!editorOpen.value || editingConfigId.value !== id) return;
    Object.assign(fields, emptyFields(), restored);
    initialEditorFields.value = JSON.stringify(fields);
    editorSuccess.value = "已还原为当前配置。";
  } catch (error) {
    editorError.value = error instanceof Error ? error.message : String(error);
  }
}
function closeEditor(): void {
  if (editorDirty.value) {
    discardPromptOpen.value = true;
    return;
  }
  editorOpen.value = false;
}
function requestDeleteConfig(configId: string): void {
  openConfigMenuId.value = null;
  if (actionsBlocked.value) return;
  if (editorDirty.value && editingConfigId.value === configId) {
    editorError.value = "请先保存或放弃当前配置修改，再删除此引擎。";
    return;
  }
  if (!configs.value.some(config => config.id === configId)) return;
  pendingDeleteIds.value = [configId];
}
function requestBulkDelete(): void {
  if (actionsBlocked.value || !selectedConfigIds.value.length) return;
  if (editorDirty.value && selectedConfigIds.value.includes(editingConfigId.value)) {
    editorError.value = "请先保存或放弃当前配置修改，再删除所选配置。";
    return;
  }
  pendingDeleteIds.value = [...selectedConfigIds.value];
}
function cancelDeleteConfig(): void {
  pendingDeleteIds.value = [];
}
function confirmDeleteConfig(): void {
  const ids = [...pendingDeleteIds.value];
  if (!ids.length || actionsBlocked.value) return;
  pendingDeleteIds.value = [];
  selectedConfigIds.value = selectedConfigIds.value.filter((id) => !ids.includes(id));
  httpTtsConfigActions.removeSelectedHttpTtsConfig(ids);
}

function selectPlaybackConfig(configId: string): void {
  openConfigMenuId.value = null;
  actionsMenuOpen.value = false;
  if (actionsBlocked.value || configId === selectedConfigId.value) return;
  if (editorDirty.value) {
    pendingSelectionId.value = configId;
    discardPromptOpen.value = true;
    return;
  }
  selectConfig(configId);
}
function discardEditor(): void {
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

function saveEditor(): void {
  if (blocked.value || !fields.name.trim() || !fields.url.trim()) return;
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
    const saved = httpTtsConfigActions.saveHttpTtsConfigFromEditor(creating ? null : editingConfigId.value, editorFields);
    if (!saved) {
      editorError.value = "保存没有完成。当前修改仍在编辑器中，请检查配置后重试。";
      return;
    }
    initialEditorFields.value = JSON.stringify(fields);
    if (creating || window.matchMedia("(max-width: 860px)").matches) {
      // A successful create must not leave the editor in "new" mode: the next save would create a duplicate.
      editorOpen.value = false;
    } else {
      editorSuccess.value = "配置已更新。";
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
  if (actionsBlocked.value) return;
  await httpTtsConfigActions.importHttpTtsConfigFromLocal();
}

async function importFromLink(): Promise<void> {
  if (!importUrl.value.trim() || actionsBlocked.value) return;
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
  if (!isMounted.value || initialDesktopOpened.value || editorOpen.value || actionsBlocked.value || !configs.value.length || !window.matchMedia("(min-width: 861px)").matches) return;
  initialDesktopOpened.value = true;
  const candidate = configs.value.find((config) => config.id === selectedConfigId.value) ?? configs.value[0];
  void openEditor(candidate.id);
}
onMounted(async () => {
  isMounted.value = true;
  const unlistenRestore = await listen<AppBootstrap>("app-state-updated", () => {
    handleRestore();
  });
  if (disposed) { unlistenRestore(); return; }
  unlisteners.push(unlistenRestore);
  openInitialDesktopEditor();
});
watch(() => configs.value.map(config => config.id), ids => {
  // Keep the editor aligned with the shared list after removing entries.
  if (editingConfigId.value && !ids.includes(editingConfigId.value) && !editorDirty.value) {
    editorOpen.value = false;
    editingConfigId.value = "";
  }
  const existingIds = new Set(ids);
  selectedConfigIds.value = selectedConfigIds.value.filter((id) => existingIds.has(id));
  pendingDeleteIds.value = pendingDeleteIds.value.filter((id) => existingIds.has(id));
});
watch(() => [configs.value.length, selectedConfigId.value, blocked.value], openInitialDesktopEditor);
</script>

<template>
  <section class="settings-card http-tts-config-card" :class="{ 'http-tts-config-card--empty': !editorOpen && configs.length === 0 }">
    <SubpageHeader class="http-tts-topbar col-span-full row-start-1" title="TTS 引擎" :back-label="embedded ? '返回阅读菜单' : '返回高级设置'" @back="returnToAdvancedSettings" >
<template #actions><div ref="addMenuTarget" class="http-tts-top-add">
        <KButton rounded inline type="button" class="gap-2"
:disabled="actionsBlocked"
          @click="addMenuOpen = !addMenuOpen; filterMenuOpen = false; actionsMenuOpen = false; openConfigMenuId = null"><PrototypeIcon name="plus"/> 添加</KButton>

      </div></template>
</SubpageHeader>
<KPopover v-if="addMenuOpen" opened :backdrop="false" :target="addMenuTarget ?? undefined" class="http-tts-add-menu w-60 max-w-[calc(100vw-32px)]">
  <div class="p-2">
<KButton rounded inline clear class="gap-2 w-full justify-start text-current" type="button" @click="addConfigAction('new')">新建配置</KButton>
<KButton rounded inline clear class="gap-2 w-full justify-start text-current" type="button" @click="addConfigAction('file')">本地 JSON 导入</KButton>
<KButton rounded inline clear class="gap-2 w-full justify-start text-current" type="button" @click="addConfigAction('url')">网络链接导入</KButton>
  </div>
</KPopover>
    <div class="http-tts-overview" :class="{ 'editor-active': editorOpen }">
    <nav class="http-tts-master-list">
      <div class="http-tts-master-summary">
        <span class="http-tts-count"><strong>{{ configs.filter(config => config.enabled).length }} 个启用</strong><small>{{ configs.length }} 个配置</small></span>
      </div>
      <ManagedListToolbar
        v-model:query="configQuery"
        search-placeholder="搜索 TTS 配置"
        :filter-expanded="filterMenuOpen"
        :actions-expanded="actionsMenuOpen"
        @toggle-filter="toggleFilterMenu"
        @toggle-actions="toggleActionsMenu"
      >
        <template #actions-menu>
          <Teleport to="body">
            <KGlass component="div" :highlight="false" v-if="actionsMenuOpen" class="http-tts-actions-menu rounded-2xl" :style="{ top: actionsMenuPosition.top + 'px', left: actionsMenuPosition.left + 'px' }">
              <KButton rounded inline clear class="gap-2 w-full justify-start text-current" type="button" @click="toggleBatchManagement">{{ batchMode ? '退出批量管理' : '批量管理' }}</KButton>
            </KGlass>
          </Teleport>
        </template>
        <template #filter-menu>
          <Teleport to="body">
            <KGlass component="div" :highlight="false" v-if="filterMenuOpen" class="http-tts-filter-menu rounded-2xl" :style="{ top: filterMenuPosition.top + 'px', left: filterMenuPosition.left + 'px' }">
              <KButton rounded inline clear class="gap-2 w-full justify-start text-current" type="button" @click="enabledFilter = 'all'; filterMenuOpen = false">全部配置</KButton>
              <KButton rounded inline clear class="gap-2 w-full justify-start text-current" type="button" @click="enabledFilter = 'enabled'; filterMenuOpen = false">仅启用</KButton>
              <KButton rounded inline clear class="gap-2 w-full justify-start text-current" type="button" @click="enabledFilter = 'disabled'; filterMenuOpen = false">仅停用</KButton>
            </KGlass>
          </Teleport>
        </template>
      </ManagedListToolbar>
      <div class="http-tts-items-scroll" @scroll.passive="openConfigMenuId = null; filterMenuOpen = false; actionsMenuOpen = false">
      <ManagedListRow name="系统朗读" subtitle="不使用远程朗读" :enabled="false"
        :busy="blocked" :show-menu="false"
        @open="selectPlaybackConfig('')" />
      <ManagedListRow v-for="config in filteredConfigs" :key="config.id"
        :name="config.name" :subtitle="config.enabled ? 'HTTP TTS' : '已停用'" :enabled="config.enabled"
        :busy="actionsBlocked"
        :batch-mode="batchMode" :selected="selectedConfigIds.includes(config.id)"
        :show-menu="!batchMode"
        @open="batchMode ? toggleConfigSelection(config.id) : openEditor(config.id)"
        @select="toggleConfigSelection(config.id)"
      >
        <template #menu-trigger>
          <KButton rounded inline clear type="button" class="http-tts-config-menu-button gap-2 size-11 p-0"
            :aria-label="`${config.name}的更多操作`"
            :disabled="actionsBlocked"
            @click.stop="toggleConfigMenu(config.id, $event)"><PrototypeIcon name="more"/></KButton>
        </template>
      </ManagedListRow>
      <p v-if="!filteredConfigs.length" class="http-tts-list-empty">{{ configs.length ? '没有符合条件的 TTS 配置' : '还没有 TTS 配置' }}</p>
      </div>
      <footer v-if="batchMode" class="http-tts-selection-bar">
        <KButton rounded inline clear type="button" class="http-tts-selection-text-button gap-2" :disabled="actionsBlocked || !filteredConfigs.length"
          @click="selectVisibleConfigs()">{{ allVisibleSelected ? '取消全选' : '全选' }}</KButton>
        <KButton rounded inline clear type="button" class="http-tts-selection-text-button gap-2" :disabled="actionsBlocked || !filteredConfigs.length"
          @click="selectVisibleConfigs(true)">反选</KButton>
        <span class="http-tts-selection-count">已选 {{ selectedConfigs.length }}</span>
        <KButton rounded inline clear type="button" class="http-tts-selection-text-button gap-2" :disabled="actionsBlocked || !selectedConfigs.length"
          @click="requestBulkExport">导出</KButton>
        <KButton rounded inline clear type="button" class="http-tts-selection-text-button http-tts-selection-delete k-color-brand-danger gap-2 " :disabled="actionsBlocked || !selectedConfigs.length"
          @click="requestBulkDelete">删除</KButton>
      </footer>
    </nav>
    <Teleport to="body">
      <KGlass component="div" :highlight="false" v-if="openConfigMenu" class="http-tts-config-menu rounded-2xl" :style="{ top: configMenuPosition.top + 'px', left: configMenuPosition.left + 'px' }">
        <KButton rounded inline clear class="gap-2 w-full justify-start text-current" type="button" :disabled="actionsBlocked || !openConfigMenu.enabled || selectedConfigId === openConfigMenu.id" @click="selectPlaybackConfig(openConfigMenu.id)">设为当前朗读</KButton>
        <KButton rounded inline clear class="k-color-brand-danger gap-2 w-full justify-start" type="button" :disabled="actionsBlocked" @click="requestDeleteConfig(openConfigMenu.id)">删除配置</KButton>
      </KGlass>
    </Teleport>
    <p v-if="!importUrlOpen && (error || (!editorOpen && editorError))" class="source-identity-confirmation">{{ editorError || error }}</p>

    </div>
    <aside v-if="!editorOpen && selectedConfig" class="http-tts-preview">
      <span class="http-tts-preview-icon"><PrototypeIcon name="tts"/></span>
      <strong>{{ selectedConfig.name }}</strong>
      <p>{{ selectedConfig.enabled ? '当前选择的网络朗读源' : '当前配置已停用' }}。参数由 Rust 保存，选择编辑后可以修改原始配置字段。</p>
      <KButton rounded inline small type="button" class="gap-2" :disabled="actionsBlocked" @click="openEditor()">编辑配置</KButton>
    </aside>
    <div v-if="importUrlOpen" class="http-tts-import-scrim" data-back-layer
      @click.self="closeUrlImport">
      <section class="http-tts-import-dialog">
        <h3 id="http-tts-import-title">从网络链接导入</h3>
        <form class="http-tts-import-url-form" @submit.prevent="importFromLink">
          <label for="http-tts-config-url">HTTP TTS 配置 JSON 链接</label>
          <input id="http-tts-config-url" v-model="importUrl" type="url" required
            inputmode="url" autocomplete="url" maxlength="4096"
            placeholder="https://example.com/http-tts.json" :disabled="importUrlBusy || actionsBlocked" />
          <KButton rounded inline type="submit" class="gap-2"
            :disabled="importUrlBusy || actionsBlocked || !importUrl.trim()">
            {{ importUrlBusy || importBusy ? '下载并导入中…' : '下载并导入' }}
          </KButton>
          <small>下载完成后会直接导入配置，上限 1 MiB。</small>
        </form>
        <p v-if="importUrlError" class="http-tts-import-error">{{ importUrlError }}</p>
        <div class="http-tts-import-actions">
          <KButton rounded inline tonal type="button" class="gap-2" :disabled="importUrlBusy || importBusy" data-back-action @click="closeUrlImport">取消</KButton>
        </div>
      </section>
    </div>
    <section v-if="editorOpen" class="http-tts-editor-page" data-back-layer>
      <article class="http-tts-editor-dialog">
        <SubpageHeader class="http-tts-editor-header col-span-full row-start-1" :title="creatingConfig ? '新建 HTTP TTS 配置' : editingConfig?.name ?? '编辑朗读源'" @back="closeEditor" local-back>
<template #actions><div class="http-tts-editor-header-actions">
            <KButton rounded inline v-if="editingConfig" type="button" class="k-color-brand-danger gap-2 " :disabled="actionsBlocked" @click="requestDeleteConfig(editingConfig.id)">删除</KButton>
            <KButton rounded inline type="button" class="gap-2" :disabled="blocked || !fields.name.trim() || !fields.url.trim()" @click="saveEditor">保存</KButton>
          </div></template>
</SubpageHeader>

        <div class="http-tts-editor-body">
            <fieldset class="http-tts-editor-fields" :disabled="blocked">
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
                  <KToggle component="span" :checked="!!fields.enabledCookieJar" @change="fields.enabledCookieJar = ($event.target as HTMLInputElement).checked"  />
                  <span><strong>启用 Cookie Jar</strong><small>允许该引擎在请求间使用 Cookie 存储</small></span>
                </label>
                <label class="http-tts-editor-toggle">
                  <KToggle component="span" :checked="!!fields.enableDangerousApi" @change="fields.enableDangerousApi = ($event.target as HTMLInputElement).checked"  />
                  <span><strong>允许危险 JS API</strong><small>仅对了解并信任来源脚本的配置开启</small></span>
                </label>
              </section>
            </fieldset>
            <div class="http-tts-editor-bottom">
              <KButton rounded inline v-if="editingConfig" type="button" class="http-tts-editor-bottom-delete k-color-brand-danger gap-2 " :disabled="actionsBlocked" @click="requestDeleteConfig(editingConfig.id)">删除配置</KButton>
              <KButton rounded inline tonal type="button" class="gap-2"
                :disabled="blocked || !editorDirty"
                @click="restoreEditorFields">还原</KButton>
              <KButton rounded inline type="button" class="gap-2" :disabled="blocked || !fields.name.trim() || !fields.url.trim()" @click="saveEditor">保存</KButton>
            </div>
            <p v-if="editorError || error" class="source-identity-confirmation">{{ editorError || error }}</p>
            <p v-if="editorSuccess" class="http-tts-editor-success">{{ editorSuccess }}</p>
        </div>
      </article>
    </section>
    <div v-if="pendingDeleteIds.length" class="http-tts-discard-mask" data-back-layer
      @click.self="cancelDeleteConfig">
      <KDialog opened :backdrop="false" class="http-tts-discard-dialog w-[min(420px,calc(100vw-32px))] max-h-[88dvh] overflow-auto">
  <template #title><h3 id="tts-delete-title">删除{{ pendingDeleteIds.length === 1 ? ' HTTP TTS 配置？' : ` ${pendingDeleteIds.length} 个 HTTP TTS 配置？` }}</h3></template>
<p>{{ pendingDeleteIds.length === 1 ? `“${pendingDeleteConfigs[0]?.name ?? '此引擎'}”将从应用中删除。` : `选中的 ${pendingDeleteIds.length} 个配置将从应用中删除。` }}此操作不能撤销，删除当前朗读源后会切回系统朗读。</p><template #buttons><KDialogButton type="button" data-back-action @click="cancelDeleteConfig">取消</KDialogButton>
<KDialogButton type="button" :disabled="actionsBlocked" @click="confirmDeleteConfig" class="k-color-brand-danger" strong>确认删除</KDialogButton></template>
</KDialog>
    </div>
    <div v-if="discardPromptOpen" class="http-tts-discard-mask" data-back-layer @click.self="keepEditor">
      <KDialog opened :backdrop="false" class="http-tts-discard-dialog w-[min(420px,calc(100vw-32px))] max-h-[88dvh] overflow-auto">
  <template #title><h3 id="tts-discard-title">放弃未保存的修改？</h3></template>
<p>当前朗读引擎的字段尚未保存。返回、切换配置或更换当前播放引擎都会丢弃未保存的修改。</p><template #buttons><KDialogButton type="button" data-back-action @click="keepEditor">继续编辑</KDialogButton>
<KDialogButton type="button" @click="discardEditor" class="k-color-brand-danger" strong>放弃修改</KDialogButton></template>
</KDialog>
    </div>
  </section>
</template>

<style scoped>
@layer components {
.http-tts-config-card {
  --tts-line: var(--app-line);
  --tts-muted: var(--app-muted);
  --tts-accent: var(--app-accent);
  display: grid;
  grid-template-columns: 300px minmax(0, 1fr);
  grid-template-rows: auto minmax(0, 1fr);
  gap: 0;
  width: 100%;
  min-width: 0;
  height: 100%;
  min-height: 0;
  padding: 0;
  border: 0;
  border-radius: 0;
  background:var(--app-background);
  color: var(--app-text);
}
.http-tts-config-card--empty{grid-template-columns:minmax(0,1fr)}
.http-tts-top-add{position:relative}
.http-tts-overview{box-sizing:border-box;grid-column:1;grid-row:2;min-width:0;height:100%;min-height:0;display:flex;flex-direction:column;overflow:hidden;padding:14px;border-right:1px solid var(--tts-line);background:var(--app-background)}
.http-tts-master-list{display:flex;flex:1 1 auto;flex-direction:column;gap:3px;min-height:0;margin:0;overflow:hidden}
.http-tts-items-scroll{display:grid;flex:1 1 auto;align-content:start;gap:3px;min-height:0;margin-right:-14px;padding-right:14px;overflow-y:auto;overscroll-behavior:contain}
.http-tts-master-summary{display:flex;align-items:center;gap:6px;padding:3px}
.http-tts-count{display:flex;flex:1;align-items:center;justify-content:space-between;gap:9px}
.http-tts-count strong,.http-tts-count small{font-size:var(--ui-font-caption);font-weight:500;color:var(--tts-muted)}
.http-tts-filter-menu{position:fixed;z-index:1000;box-sizing:border-box;display:grid;width:min(170px,calc(100vw - 16px));max-height:calc(100vh - 16px);overflow-y:auto;padding:5px;border:1px solid var(--tts-line,var(--app-line));border-radius:11px;background:var(--app-surface);box-shadow:0 12px 28px #22223322}
.http-tts-filter-menu button{width:100%;min-height:34px;padding:0 10px;border:0;border-radius:7px;background:transparent;color:var(--app-text);text-align:left;font:inherit;font-size:var(--ui-font-body);cursor:pointer}
.http-tts-actions-menu{position:fixed;z-index:1000;box-sizing:border-box;display:grid;width:min(180px,calc(100vw - 16px));max-height:calc(100vh - 16px);overflow-y:auto;padding:5px;border:1px solid var(--tts-line,var(--app-line));border-radius:11px;background:var(--app-surface);box-shadow:0 12px 28px #22223322}
.http-tts-actions-menu button{width:100%;min-height:34px;padding:0 10px;border:0;border-radius:7px;background:transparent;color:var(--app-text);text-align:left;font:inherit;font-size:var(--ui-font-body);cursor:pointer}
.http-tts-actions-menu button:disabled{opacity:.4;cursor:default}
.http-tts-selection-bar{display:flex;flex:none;flex-wrap:wrap;align-items:center;gap:5px;margin-top:auto;padding:8px 3px;border-top:1px solid var(--tts-line);background:var(--app-background)}
.http-tts-selection-text-button{min-height:30px;padding:0 7px;border:0;border-radius:7px;background:transparent;color:var(--app-muted);font:inherit;font-size:var(--ui-font-body);cursor:pointer}
.http-tts-selection-text-button:disabled{opacity:.45;cursor:default}
.http-tts-selection-count{margin-left:auto;color:var(--tts-muted);font-size:var(--ui-font-caption)}
.http-tts-selection-delete{color:var(--app-danger)}
.http-tts-config-menu-button{display:grid;place-items:center;width:30px;height:34px;padding:0;border:0;border-radius:8px;background:transparent;color:var(--app-muted);cursor:pointer}
.http-tts-config-menu-button:disabled{opacity:.45;cursor:default}
.http-tts-config-menu-button :deep(svg){width:17px;height:17px}
.http-tts-config-menu{position:fixed;z-index:1000;box-sizing:border-box;width:min(190px,calc(100vw - 16px));max-height:calc(100vh - 16px);overflow-y:auto;padding:5px;border:1px solid var(--tts-line,var(--app-line));border-radius:11px;background:var(--app-surface);box-shadow:0 12px 28px #22223322}
.http-tts-config-menu button{width:100%;min-height:34px;padding:0 10px;border:0;border-radius:7px;background:transparent;color:var(--app-text);text-align:left;font:inherit;font-size:var(--ui-font-body);cursor:pointer}
.http-tts-config-menu button:disabled{opacity:.4;cursor:default}
.http-tts-list-empty{margin:0;padding:24px 10px;color:var(--tts-muted);font-size:var(--ui-font-caption);text-align:center}
.http-tts-import-scrim{position:fixed;z-index:2000;inset:0;display:grid;place-items:center;padding:16px;background:var(--app-scrim)}
.http-tts-import-dialog{box-sizing:border-box;width:min(420px,100%);max-height:min(82dvh,700px);overflow:auto;padding:22px;border:1px solid var(--tts-line);border-radius:14px;background:var(--app-surface);box-shadow:0 20px 40px #11112233}
.http-tts-import-dialog h3{margin:0;font-size:var(--ui-font-heading)}
.http-tts-import-url-form{display:grid;gap:9px;margin:10px 0 14px}
.http-tts-import-url-form label{font-size:var(--ui-font-body);font-weight:650}
.http-tts-import-url-form input{box-sizing:border-box;width:100%;min-height:39px;padding:9px 11px;border:1px solid var(--tts-line);border-radius:9px;background:var(--app-surface);color:var(--app-text);font:inherit;font-size:var(--ui-font-body)}
.http-tts-import-url-form button{justify-self:start}
.http-tts-import-url-form small{color:var(--tts-muted);font-size:var(--ui-font-caption);line-height:1.5}
.http-tts-import-error{margin:9px 0 18px;color:var(--app-danger);font-size:var(--ui-font-caption);line-height:1.5}
.http-tts-import-actions{display:flex;justify-content:flex-end;gap:8px}
.http-tts-overview>.source-identity-confirmation{margin-top:10px}
.http-tts-preview{grid-column:2;grid-row:2;display:grid;align-content:center;justify-items:center;gap:12px;min-height:300px;padding:20px;background:var(--app-background);color:var(--tts-muted);text-align:center}
.http-tts-preview-icon{display:grid;place-items:center;width:44px;height:44px;border-radius:12px;background:var(--app-accent-soft);color:var(--tts-accent)}
.http-tts-preview-icon :deep(svg){width:24px;height:24px}
.http-tts-preview strong{font-size:var(--ui-font-caption);color:var(--app-text)}
.http-tts-preview p{max-width:290px;margin:0;color:var(--app-subtle);font-size:var(--ui-font-caption);line-height:1.65}
.http-tts-editor-page{grid-column:2;grid-row:2;min-width:0;min-height:0;overflow-y:auto;background:var(--app-background)}
.http-tts-editor-dialog{width:100%;background:var(--app-background)}
.http-tts-editor-body{padding:0 34px 48px}
.http-tts-editor-loading{padding:28px 0;color:var(--tts-muted);font-size:var(--ui-font-caption);text-align:center}
.http-tts-editor-fields{display:block;min-width:0;margin:0;padding:0;border:0}
.http-tts-editor-section{padding:18px 0;border-top:1px solid var(--tts-line)}
.http-tts-editor-section>header{display:grid;gap:3px;margin-bottom:12px}
.http-tts-editor-section>header strong{font-size:var(--ui-font-caption);color:var(--app-text)}
.http-tts-editor-grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:0 12px}
.http-tts-editor-grid .form-field:nth-child(2){grid-column:1/-1}
.http-tts-editor-body .form-field{display:grid;gap:6px;margin:0 0 14px;min-width:0}
.http-tts-editor-body .form-field>span{font-size:var(--ui-font-caption);font-weight:600;color:var(--app-muted)}
.http-tts-editor-body .form-field>small{font-size:var(--ui-font-caption);line-height:1.6;color:var(--tts-muted)}
.http-tts-editor-body input:not([type=checkbox]),.http-tts-editor-body textarea{box-sizing:border-box;width:100%;min-width:0;min-height:36px;padding:8px 9px;border:1px solid var(--tts-line);border-radius:9px;background:var(--app-surface);color:var(--app-text);font-size:var(--ui-font-body);line-height:1.5}
.http-tts-editor-body textarea{min-height:88px;resize:vertical;font-family:ui-monospace,SFMono-Regular,Consolas,monospace}
.http-tts-editor-toggle{display:flex;align-items:flex-start;gap:11px;padding:11px 0;border-bottom:1px solid var(--app-line);cursor:pointer}
.http-tts-editor-toggle input{flex:none;width:17px;height:17px;margin:2px 0 0;accent-color:var(--tts-accent)}
.http-tts-editor-toggle>span{display:grid;gap:4px;min-width:0}
.http-tts-editor-toggle strong{font-size:var(--ui-font-caption);color:var(--app-text)}
.http-tts-editor-toggle small{font-size:var(--ui-font-caption);color:var(--tts-muted);line-height:1.5}
.http-tts-editor-bottom{display:flex;align-items:center;justify-content:flex-end;gap:8px;padding-top:17px;border-top:1px solid var(--tts-line)}
.http-tts-editor-bottom-delete{margin-right:auto}
.http-tts-editor-success{margin:10px 0;padding:10px 12px;border-radius:9px;background:var(--app-success-soft);color:var(--app-success);font-size:var(--ui-font-caption)}
.http-tts-discard-mask{position:fixed;z-index:100;inset:0;display:grid;place-items:center;padding:16px;background:var(--app-scrim)}
@media(max-width:860px){
 .http-tts-config-card{display:flex;flex-direction:column;height:100%;min-height:0}
 .http-tts-overview{flex:1 1 auto;height:auto;min-height:0;max-height:none;padding:16px var(--ui-page-gutter);border:0;background:var(--app-background)}
 .http-tts-preview{display:none}
 .http-tts-config-card:has(.http-tts-editor-page) .http-tts-topbar,
 .http-tts-config-card:has(.http-tts-editor-page) .http-tts-overview{display:none}
 .http-tts-editor-body{padding:24px var(--ui-page-gutter) calc(32px + var(--safe-bottom))}
 .http-tts-editor-page{flex:1 1 auto;min-height:0;overflow-y:auto}
 .http-tts-editor-grid{grid-template-columns:1fr}
 .http-tts-editor-grid .form-field:nth-child(2){grid-column:auto}
 .http-tts-editor-bottom{display:none}
}
}
</style>

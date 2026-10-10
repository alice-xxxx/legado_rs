<script setup lang="ts">
import SourceDebugPanel from "./SourceDebugPanel.vue";
import SourceManagerPage from "./SourceManagerPage.vue";
import SourceMetadataEditorPage from "./SourceMetadataEditorPage.vue";
import SourceFormEditor from "./SourceFormEditor.vue";
import { useSourceLoginActions } from "./useSourceLoginActions";
import { useSourceDebugActions } from "./useSourceDebugActions";
import { useSourceManagementActions } from "./useSourceManagementActions";
import type { AppBootstrap, AppTask, ResourceDescriptor, SourceMetadata, SourceLoginForm, TasksResource } from "../../api/types";
import { computed, onBeforeUnmount, onMounted, proxyRefs, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { appBootstrap } from "../../api/core";
import { readResource } from "../../api/resources";
import { tasksResource } from "../../api/tasks";
import { notify } from "../../app/notifications";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { clearSourceDebugPreview, debugSearchResult, searchBooks } from "../../api/search";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import { onBeforeRouteLeave, useRoute } from "vue-router";
import router, { routeNames } from "../../router";



const errorText = (error: unknown): string => error instanceof Error ? error.message : String(error);

const sources = ref<SourceMetadata[]>([]);
const sourceDefinitionsResource = ref<ResourceDescriptor | null>(null);
const selectedSourceIds = ref<string[]>([]);
const selectedSourceRows = ref<string[]>([]);
const pendingSourceRemoval = ref<string[]>([]);
const editingSource = ref<SourceMetadata | null>(null);
const sourceBatchBusy = ref(false);
const sourceExportBusy = ref(false);
const sourceImportBusy = ref(false);
const sourceImportOpen = ref(false);
const sourceImportUrl = ref("");
const sourceLoading = ref(true);
const sourceLoaded = ref(false);
const sourceLoadError = ref("");
const recoveryRequired = computed(() => shelfBatchRecoveryRequired.value);
const taskList = ref<AppTask[]>([]);
const taskListLoaded = ref(false);
const searchBusy = computed(() => !taskListLoaded.value || taskList.value.some(task => task.kind === "search"
  && ["queued", "running", "pausing", "cancelling"].includes(task.status)));
const searchResultBatchBusy = ref(false);
let sourceSnapshotRevision = 0;
let restoreFrontendRevision = 0;
let sourceRefreshRequestRevision = 0;
let taskRefreshRequestRevision = 0;
let unlisteners: UnlistenFn[] = [];
let pageDisposed = false;

const enabledSources = computed(() => sources.value.filter(source => source.enabled));
const bookSourceCandidates = computed(() => enabledSources.value.filter(source => source.isRss !== true));
function applySourcesSnapshot(nextSources: SourceMetadata[]): void {
  sourceSnapshotRevision += 1;
  sources.value = nextSources;
  selectedSourceIds.value = selectedSourceIds.value.filter(id => bookSourceCandidates.value.some(source => source.id === id));
  selectedSourceRows.value = selectedSourceRows.value.filter(id => sources.value.some(source => source.id === id));
}

const sourceManagementActions = useSourceManagementActions({
  sources,
  selectedSourceIds,
  selectedSourceRows,
  bookSourceCandidates,
  sourceBatchBusy,
  sourceExportBusy,
  sourceImportOpen,
  sourceImportBusy,
  sourceImportUrl,
  pendingSourceRemoval,
  editingSource,
  sourceDefinitionsResource,
  recoveryRequired,
  advanceSourceSnapshotRevision: () => ++sourceSnapshotRevision,
  getSourceSnapshotRevision: () => sourceSnapshotRevision,
  getRestoreFrontendRevision: () => restoreFrontendRevision,
  applySourcesSnapshot,
  errorText,
  notify,
});
const sourceDebugActions = useSourceDebugActions({
  sources,
  bookSourceCandidates,
  selectedSourceRows,
  sourceBatchBusy,
  sourceExportBusy,
  searchBusy,
  searchResultBatchBusy,
  shelfBatchRecoveryRequired: recoveryRequired,
  searchBooks,
  debugSearchResult,
  clearSourceDebugPreview,
  readResource,
  notify,
  errorText,
});
watch(sources, () => {
  const target = sourceDebugActions.sourceDebugTarget.value;
  if (target && !bookSourceCandidates.value.some(source => source.id === target.id)) {
    sourceDebugActions.closeSourceDebug();
  }
});
const sourceLoginTarget = ref<SourceMetadata | null>(null);
const sourceLoginBusy = ref(false);
const sourceHasLoginState = ref<boolean | null>(null);
const sourceLoginError = ref("");
const sourceLoginForm = ref<SourceLoginForm | null>(null);
const sourceLoginWebSessionId = ref("");
const sourceLoginCredentials = ref<Record<string, string>>(Object.create(null) as Record<string, string>);
const sourceLoginFormError = ref("");
const sourceLoginFeedback = ref("");
const sourceLoginActions = useSourceLoginActions({
  sources,
  selectedSourceRows,
  sourceBatchBusy,
  sourceExportBusy,
  shelfBatchRecoveryRequired: recoveryRequired,
  sourceLoginTarget,
  sourceLoginBusy,
  sourceHasLoginState,
  sourceLoginError,
  sourceLoginForm,
  sourceLoginWebSessionId,
  sourceLoginCredentials,
  sourceLoginFormError,
  sourceLoginFeedback,
  notify,
  errorText,
});

function toggleSourceRow(id: string): void {
  selectedSourceRows.value = selectedSourceRows.value.includes(id)
    ? selectedSourceRows.value.filter(item => item !== id)
    : [...selectedSourceRows.value, id];
}
function sourceName(id: string): string {
  return sources.value.find(source => source.id === id)?.name ?? id;
}

async function refreshSources(descriptor?: ResourceDescriptor): Promise<void> {
  const requestRevision = ++sourceRefreshRequestRevision;
  const restoreRevision = restoreFrontendRevision;
  sourceLoading.value = true;
  sourceLoadError.value = "";
  try {
    await sourceManagementActions.refreshSources(descriptor);
    if (requestRevision !== sourceRefreshRequestRevision || restoreRevision !== restoreFrontendRevision) return;
    sourceLoaded.value = true;
  } catch (error) {
    if (requestRevision === sourceRefreshRequestRevision && restoreRevision === restoreFrontendRevision) {
      sourceLoadError.value = errorText(error);
    }
    throw error;
  } finally {
    if (requestRevision === sourceRefreshRequestRevision && restoreRevision === restoreFrontendRevision) {
      sourceLoading.value = false;
    }
  }
}

async function refreshTasks(descriptor?: ResourceDescriptor): Promise<void> {
  const requestRevision = ++taskRefreshRequestRevision;
  const restoreRevision = restoreFrontendRevision;
  const resource = descriptor ?? (await tasksResource()).resource;
  const document = await readResource<TasksResource>(resource);
  if (pageDisposed || requestRevision !== taskRefreshRequestRevision || restoreRevision !== restoreFrontendRevision) return;
  taskList.value = Array.isArray(document.tasks) ? document.tasks : [];
  taskListLoaded.value = true;
}

function reportTaskRefreshError(error: unknown): void {
  if (!pageDisposed) notify(`浠诲姟鐘舵€佸埛鏂板け璐ワ細${errorText(error)}`, "error");
}

async function loadSourcesFromBootstrap(): Promise<void> {
  const restoreRevision = restoreFrontendRevision;
  const requestRevision = ++sourceRefreshRequestRevision;
  sourceLoading.value = true;
  sourceLoadError.value = "";
  try {
    const bootstrap = await appBootstrap();
    if (restoreRevision !== restoreFrontendRevision || requestRevision !== sourceRefreshRequestRevision) return;
    await refreshSources(bootstrap.sources);
    if (restoreRevision === restoreFrontendRevision) {
      selectedSourceIds.value = bookSourceCandidates.value.map(source => source.id);
    }
  } catch (error) {
    if (restoreRevision === restoreFrontendRevision && requestRevision === sourceRefreshRequestRevision) {
      sourceLoadError.value = errorText(error);
    }
  } finally {
    if (restoreRevision === restoreFrontendRevision && requestRevision === sourceRefreshRequestRevision) {
      sourceLoading.value = false;
    }
  }
}

async function refreshSourceList(): Promise<void> {
  try {
    if (!sourceDefinitionsResource.value) {
      await loadSourcesFromBootstrap();
      return;
    }
    await refreshSources();
  } catch (error) {
    sourceLoadError.value = errorText(error);
  }
}

const management = proxyRefs({
  ...sourceManagementActions,
  refreshSourceList,
  sources,
  enabledSources,
  bookSourceCandidates,
  selectedSourceRows,
  sourceBatchBusy,
  sourceExportBusy,
  sourceImportOpen,
  sourceImportBusy,
  sourceImportUrl,
  pendingSourceRemoval,
  editingSource,
  sourceName,
  toggleSourceRow,
});
const debug = proxyRefs(sourceDebugActions);
const login = proxyRefs({
  ...sourceLoginActions,
  sourceLoginTarget,
  sourceLoginBusy,
  sourceHasLoginState,
  sourceLoginError,
  sourceLoginForm,
  sourceLoginWebSessionId,
  sourceLoginCredentials,
  sourceLoginFormError,
  sourceLoginFeedback,
});

async function handleAppStateUpdated(bootstrap: AppBootstrap): Promise<void> {
  const restoreRevision = ++restoreFrontendRevision;
  sourceRefreshRequestRevision += 1;
  taskRefreshRequestRevision += 1;
  taskListLoaded.value = false;
  sourceSnapshotRevision += 1;
  sourceDefinitionsResource.value = null;
  sourceLoaded.value = false;
  selectedSourceIds.value = [];
  selectedSourceRows.value = [];
  pendingSourceRemoval.value = [];
  editingSource.value = null;
  sourceDebugActions.closeSourceDebug();
  sourceLoginActions.closeSourceLogin();
  await Promise.all([refreshSources(bootstrap.sources), refreshTasks()]);
  if (pageDisposed || restoreRevision !== restoreFrontendRevision) return;
  selectedSourceIds.value = bookSourceCandidates.value.map(source => source.id);
}

onMounted(async () => {
  try {
    const unlistenSources = await listen<ResourceDescriptor>("sources-updated", event => {
      void refreshSources(event.payload).catch(error => { sourceLoadError.value = errorText(error); });
    });
    if (pageDisposed) unlistenSources();
    else unlisteners.push(unlistenSources);
    const unlistenTaskResource = await listen<{ kind: string; resource: ResourceDescriptor }>("resource-updated", event => {
      if (event.payload.kind === "tasks") {
        void refreshTasks(event.payload.resource).catch(reportTaskRefreshError);
      }
    });
    if (pageDisposed) unlistenTaskResource();
    else unlisteners.push(unlistenTaskResource);
    const unlistenTask = await listen<{ taskId: string; resource: ResourceDescriptor }>("task-updated", event => {
      void refreshTasks(event.payload.resource).catch(reportTaskRefreshError);
    });
    if (pageDisposed) unlistenTask();
    else unlisteners.push(unlistenTask);
    const unlistenAppState = await listen<AppBootstrap>("app-state-updated", event => {
      void handleAppStateUpdated(event.payload).catch(error => { sourceLoadError.value = errorText(error); });
    });
    if (pageDisposed) unlistenAppState();
    else unlisteners.push(unlistenAppState);
  } catch (error) {
    sourceLoadError.value = errorText(error);
  }
  if (pageDisposed) return;
  await Promise.all([
    loadSourcesFromBootstrap(),
    refreshTasks().catch(error => {
      if (!pageDisposed) notify(`浠诲姟鐘舵€佽鍙栧け璐ワ細${errorText(error)}`, "error");
    }),
  ]);
});

onBeforeUnmount(() => {
  pageDisposed = true;
  sourceRefreshRequestRevision += 1;
  sourceSnapshotRevision += 1;
  taskRefreshRequestRevision += 1;
  restoreFrontendRevision += 1;
  sourceDebugActions.closeSourceDebug();
  sourceLoginActions.closeSourceLogin();
  for (const unlisten of unlisteners) unlisten();
  unlisteners = [];
});

function returnFromSources(): void {
  if (router.options.history.state.back) void router.back();
  else void router.replace({ name: routeNames.shelf });
}

const sourceImportMode = ref<"new"|"url">("url");
let requestedSourceImportMode: "new" | "url" | null = null;
const currentRoute = useRoute();
const routeSourceImportMode = computed(() => currentRoute.name === routeNames.sources
  && (currentRoute.query.sourceImport === "url" || currentRoute.query.sourceImport === "new")
  ? currentRoute.query.sourceImport : "");
let sourceImportQueryOpen = false;
let suppressSourceImportQuerySync = false;
function syncSourceImportRoute(open: boolean): void {
  if (currentRoute.name !== routeNames.sources) return;
  if (open && routeSourceImportMode.value) return;
  const query = { ...currentRoute.query };
  if (open) {
    query.sourceImport = sourceImportMode.value;
    if (sourceImportMode.value === "new") delete query.importUrl;
  } else {
    delete query.sourceImport;
    delete query.importUrl;
  }
  void router.replace({ name: routeNames.sources, query });
}
watch(routeSourceImportMode, mode => {
  if (mode) {
    sourceImportQueryOpen = true;
    requestedSourceImportMode = mode;
    sourceImportMode.value = mode;
    if (mode === "url") {
      const url = currentRoute.query.importUrl;
      sourceImportUrl.value = typeof url === "string" ? url : "";
    } else sourceImportUrl.value = "";
    sourceImportOpen.value = true;
    return;
  }
  if (!sourceImportQueryOpen) return;
  sourceImportQueryOpen = false;
  if (sourceImportOpen.value) {
    suppressSourceImportQuerySync = true;
    sourceImportOpen.value = false;
    suppressSourceImportQuerySync = false;
  }
}, { immediate: true });
watch(sourceImportOpen, (opened, wasOpen) => {
  if (suppressSourceImportQuerySync) return;
  if (opened) syncSourceImportRoute(true);
  else if (wasOpen) syncSourceImportRoute(false);
}, { flush: "sync" });
const newSourceTemplate = `{
  "bookSourceName": "",
  "bookSourceUrl": "",
  "bookSourceEnabled": true,
  "bookSourceType": 0
}`;
const newSourceJson = ref(newSourceTemplate);
const newSourceEditorMode = ref<"form" | "json">("form");
const newSourceFormPending = ref(false);
const newSourceDiscardOpen = ref(false);
const hasNewSourceDraft = computed(() => newSourceFormPending.value || newSourceJson.value !== newSourceTemplate);
let pendingSourceImportNavigation: ((allow: boolean) => void) | null = null;
const newSourceError = computed(() => {
  try {
    const parsed: unknown = JSON.parse(newSourceJson.value);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return "配置必须是 JSON 对象";
    const source = parsed as Record<string, unknown>;
    if (typeof source.bookSourceName !== "string" || !source.bookSourceName.trim()) return "需要填写 bookSourceName（来源名称）";
    if (typeof source.bookSourceUrl !== "string" || !source.bookSourceUrl.trim()) return "需要填写 bookSourceUrl（来源地址）";
    return "";
  } catch (error) { return error instanceof Error ? error.message : String(error); }
});
async function submitNewSource(): Promise<void> {
  if (importLocked.value || newSourceError.value || newSourceFormPending.value) return;
  const created = await management.createSourceFromJson(newSourceJson.value);
  if (created) {
    newSourceJson.value = newSourceTemplate;
    newSourceFormPending.value = false;
  }
}
const importLocked = computed(() => recoveryRequired.value || sourceImportBusy.value
  || sourceExportBusy.value || sourceBatchBusy.value
  || sourceManagementActions.sourceRemovalBusy.value || sourceManagementActions.sourceEditBusy.value);
const urlImportError = computed(() => {
  const input = sourceImportUrl.value.trim();
  if (!input) return "";
  if (!/^https?:\/\//i.test(input)) return "请输入以 http:// 或 https:// 开头的完整链接";
  try {
    const url = new URL(input);
    return (url.protocol === "http:" || url.protocol === "https:") && url.hostname
      ? "" : "仅支持 HTTP 或 HTTPS 来源链接";
  } catch { return "链接格式不正确"; }
});
function openSourceImport(mode: "new" | "url"): void {
  if (importLocked.value) return;
  sourceImportMode.value = mode;
  requestedSourceImportMode = mode;
  if (mode === "url") sourceImportUrl.value = "";
  sourceImportOpen.value = true;
}
function closeSourceImport(): void {
  if (importLocked.value) return;
  if (hasNewSourceDraft.value) {
    newSourceDiscardOpen.value = true;
    return;
  }
  requestedSourceImportMode = null;
  sourceImportOpen.value = false;
}
function keepNewSourceDraft(): void {
  newSourceDiscardOpen.value = false;
  const resume = pendingSourceImportNavigation;
  pendingSourceImportNavigation = null;
  resume?.(false);
}
function abandonNewSource(): void {
  if (importLocked.value) return;
  newSourceDiscardOpen.value = false;
  newSourceJson.value = newSourceTemplate;
  newSourceEditorMode.value = "form";
  newSourceFormPending.value = false;
  requestedSourceImportMode = null;
  const resume = pendingSourceImportNavigation;
  pendingSourceImportNavigation = null;
  if (resume) suppressSourceImportQuerySync = true;
  sourceImportOpen.value = false;
  if (resume) suppressSourceImportQuerySync = false;
  resume?.(true);
}
function submitSourceImport(): void {
  if (importLocked.value || !sourceImportUrl.value.trim() || urlImportError.value) return;
  void sourceManagementActions.importSourceFromUrl();
}
watch(sourceImportOpen, opened => {
  if (opened) {
    sourceImportMode.value = requestedSourceImportMode ?? "url";
    requestedSourceImportMode = null;
  }
});

const pendingDraft = ref(false);
const confirmRouteLeave = ref(false);
const pendingSourceSwitch = ref<SourceMetadata | null>(null);
let initialDesktopSourceOpened = false;
watch(() => sources.value.length, count => {
  if (initialDesktopSourceOpened || !count || editingSource.value
      || sourceImportOpen.value || typeof window === "undefined"
      || window.innerWidth <= 860) return;
  initialDesktopSourceOpened = true;
  const firstSource = sources.value[0];
  if (firstSource) void sourceManagementActions.beginSourceEdit(firstSource);
}, { immediate: true });
function requestEditorSwitch(source: SourceMetadata): void {
  if (sourceManagementActions.sourceEditBusy.value || recoveryRequired.value) return;
  if (!editingSource.value) {
    void sourceManagementActions.beginSourceEdit(source);
    return;
  }
  if (editingSource.value.id === source.id) return;
  if (isEdited()) {
    pendingSourceSwitch.value = source;
    confirmRouteLeave.value = true;
    return;
  }
  sourceManagementActions.closeSourceEdit();
  void sourceManagementActions.beginSourceEdit(source);
}
function runEditorAction(action: "test" | "login" | "export" | "remove"): void {
  const source = editingSource.value;
  if (!source || recoveryRequired.value || sourceManagementActions.sourceEditBusy.value) return;
  if (action === "remove") {
    pendingSourceRemoval.value = [source.id];
    return;
  }
  selectedSourceRows.value = [source.id];
  if (action === "test") sourceDebugActions.openSelectedSourceDebug();
  else if (action === "login") void sourceLoginActions.openSelectedSourceLogin();
  else void sourceManagementActions.exportSelectedSources();
}
let resolveNavigation: ((allow:boolean)=>void) | null = null;

function isEdited(): boolean {
  if (pendingDraft.value) return true;
  const current = sourceManagementActions.sourceEditJson.value;
  const original = sourceManagementActions.sourceEditOriginalJson.value;
  return current !== original;
}
function chooseRouteLeave(allow:boolean): void {
  const resolve = resolveNavigation;
  const target = pendingSourceSwitch.value;
  resolveNavigation = null;
  pendingSourceSwitch.value = null;
  confirmRouteLeave.value = false;
  if (allow) {
    sourceManagementActions.closeSourceEdit();
    if (target) void sourceManagementActions.beginSourceEdit(target);
  }
  resolve?.(allow);
}
onBeforeRouteLeave(() => {
  if (sourceImportOpen.value) {
    if (importLocked.value) return false;
    if (hasNewSourceDraft.value) {
      return new Promise<boolean>(resolve => {
        pendingSourceImportNavigation?.(false);
        pendingSourceImportNavigation = resolve;
        newSourceDiscardOpen.value = true;
      });
    }
    suppressSourceImportQuerySync = true;
    sourceImportOpen.value = false;
    suppressSourceImportQuerySync = false;
  }
  if (!editingSource.value) return true;
  if (sourceManagementActions.sourceEditBusy.value) return false;
  if (!isEdited()) {
    sourceManagementActions.closeSourceEdit();
    return true;
  }
  // Router back/side navigation cannot bypass the editor's own Cancel dialog.
  return new Promise<boolean>(resolve => {
    resolveNavigation?.(false);
    resolveNavigation = resolve;
    confirmRouteLeave.value = true;
  });
});
onBeforeUnmount(() => {
  resolveNavigation?.(false);
  pendingSourceImportNavigation?.(false);
});
</script>

<template>
  <section v-if="!sourceLoaded" class="source-data-state">
    <h2>{{ sourceLoading ? '正在读取书源' : '暂时无法读取书源' }}</h2>
    <p>{{ sourceLoading ? '正在读取书源列表和配置。' : sourceLoadError || '请检查应用状态后重试。' }}</p>
    <button type="button" class="button primary" :disabled="sourceLoading" @click="loadSourcesFromBootstrap">
      {{ sourceLoading ? '加载中…' : '重新加载' }}
    </button>
  </section>
  <template v-else>
  <p v-if="sourceLoadError" class="source-load-warning">
    书源刷新失败：{{ sourceLoadError }}
    <button type="button" class="text-button" :disabled="sourceLoading" @click="refreshSourceList">重试</button>
  </p>
  <div class="source-workspace-frame" :class="{ 'source-workspace-frame--editing': !!management.editingSource }">
  <SourceManagerPage
    :editor-open="!!management.editingSource"
    :editing-source-id="management.editingSource?.id"
    :sources="management.sources"
    :selected-ids="management.selectedSourceRows"
    :batch-busy="management.sourceBatchBusy || management.sourceEditBusy || management.sourceRemovalBusy"
    :export-busy="management.sourceExportBusy"
    :locked="recoveryRequired"
    :search-busy="searchBusy"
    @selection-change="management.selectedSourceRows = $event"
    @toggle-source="management.toggleSourceRow"
    @change-enabled="management.changeSourceEnabled"
    @refresh="management.refreshSourceList"
    @create-open="openSourceImport('new')"
    @import-local="management.importSourceFileFromPicker()"
    @import-url-open="openSourceImport('url')"
    @login="login.openSelectedSourceLogin"
    @debug="debug.openSelectedSourceDebug"
    @set-group="management.setSelectedSourcesGroup"
    @set-enabled="management.setSelectedSourcesEnabled"
    @export="management.exportSelectedSources"
    @remove="management.pendingSourceRemoval = $event"
    @edit="management.beginSourceEdit"
    @switch-editor="requestEditorSwitch"
    @back="returnFromSources"
  >
    <template #editor>
      <SourceMetadataEditorPage
        v-if="management.editingSource"
        :key="management.editingSource.id"
        :source="management.editingSource"
        v-model:json-text="management.sourceEditJson"
        :original-json="management.sourceEditOriginalJson"
        :original-url="management.sourceEditInitialUrl"
        :busy="management.sourceEditBusy"
        :locked="recoveryRequired"
        @save="management.saveSourceDefinitionJson"
        @cancel="management.closeSourceEdit"
        @pending-draft="pendingDraft = $event"
        @test="runEditorAction('test')"
        @login="runEditorAction('login')"
        @export="runEditorAction('export')"
        @remove="runEditorAction('remove')"
      />
    </template>
  </SourceManagerPage>
  </div>

    <section v-if="confirmRouteLeave" class="modal-backdrop source-leave-guard"
>
      <article class="app-modal">
        <h2>{{ pendingSourceSwitch ? '切换编辑来源？' : '放弃书源修改？' }}</h2>
        <p class="modal-description">当前 JSON 有尚未保存的修改。{{ pendingSourceSwitch ? '切换来源' : '离开页面' }}会丢弃全部草稿，包括正在编辑的字段。</p>
        <div class="modal-actions">
          <button type="button" class="button secondary" @click="chooseRouteLeave(false)">继续编辑</button>
          <button type="button" class="button danger" @click="chooseRouteLeave(true)">{{ pendingSourceSwitch ? '放弃并切换' : '放弃并离开' }}</button>
        </div>
      </article>
    </section>

    <section v-if="management.sourceImportOpen" class="modal-backdrop" @click.self="closeSourceImport">
      <article class="app-modal source-import-modal" :class="{ 'source-import-modal--url': sourceImportMode === 'url', 'source-import-modal--new': sourceImportMode === 'new' }"
@keydown.esc="closeSourceImport">
        <header class="source-import-heading">
          <h2 id="source-import-title">{{ sourceImportMode === 'new' ? '新建来源' : '从网络链接导入' }}</h2>
          <button type="button" class="source-import-close"
            :disabled="importLocked" @click="closeSourceImport"><PrototypeIcon name="close"/></button>
        </header>
        <div v-if="sourceImportMode==='new'" class="source-import-new">
          <div class="source-import-new-tabs">
            <button type="button" :class="{active:newSourceEditorMode==='form'}"
              :disabled="importLocked || newSourceFormPending" @click="newSourceEditorMode='form'">配置表单</button>
            <button type="button" :class="{active:newSourceEditorMode==='json'}"
              :disabled="importLocked || newSourceFormPending" @click="newSourceEditorMode='json'">完整 JSON</button>
          </div>
          <SourceFormEditor v-if="newSourceEditorMode==='form'" :json-text="newSourceJson" :enabled="true"
            :disabled="importLocked" @update:json-text="newSourceJson = $event" @pending="newSourceFormPending = $event" />
          <label v-else for="source-import-new-json">书源定义 JSON
            <textarea id="source-import-new-json" v-model="newSourceJson"
              spellcheck="false" autocapitalize="off" autocomplete="off" :disabled="importLocked" rows="12"></textarea>
          </label>
          <p class="source-import-hint">所有 Legado 字段和嵌套解析规则均保留原始类型。配置表单里的修改需先点击「应用修改」，再保存来源。</p>
          <p v-if="newSourceError" class="source-import-error">{{ newSourceError }}</p>
          <p v-if="newSourceFormPending" class="source-import-hint">表单有尚未应用的修改，请先应用或撤销。</p>
          <p class="source-import-hint">与现有来源地址相同时将更新已有配置，而不是生成重复来源。</p>
          <div class="source-import-actions">
            <button type="button" class="button secondary" :disabled="importLocked" @click="closeSourceImport">取消</button>
            <button type="button" class="button primary"
              :disabled="importLocked || !!newSourceError || newSourceFormPending" @click="submitNewSource">
              {{ management.sourceImportBusy ? '正在保存…' : '保存来源配置' }}
            </button>
          </div>
        </div>
        <form v-else class="source-import-url" @submit.prevent="submitSourceImport">
          <label for="source-import-url">内容来源 JSON 链接</label>
          <input id="source-import-url" v-model="management.sourceImportUrl"
            type="url" inputmode="url" autocomplete="url" maxlength="4096"
            placeholder="https://example.com/source.json"
            :class="{ 'is-invalid': !!urlImportError }" :disabled="importLocked"/>
          <button type="submit" class="button primary"
            :disabled="importLocked || !management.sourceImportUrl.trim() || !!urlImportError">
            {{ management.sourceImportBusy ? '下载并导入中…' : '下载并导入' }}
          </button>
          <small>下载完成后会直接导入来源，仅支持 HTTP(S)，上限 32 MiB。</small>
          <p v-if="urlImportError" class="source-import-error">{{ urlImportError }}</p>
          <div class="source-import-actions">
            <button type="button" class="button secondary" :disabled="importLocked" @click="closeSourceImport">取消</button>
          </div>
        </form>
        <p v-if="management.sourceImportBusy" class="source-import-loading">{{ sourceImportMode === 'new' ? '正在保存来源配置…' : '正在下载并导入来源…' }}</p>
      </article>
    </section>
    <div v-if="newSourceDiscardOpen" class="modal-backdrop source-new-discard-mask">
      <article class="app-modal">
        <h2 id="source-new-discard-title">放弃新建来源的修改？</h2>
        <p class="modal-description">尚未保存的名称、网络参数和解析规则将被丢弃。关闭或离开会丢失全部草稿。</p>
        <div class="modal-actions">
          <button type="button" class="button secondary" @click="keepNewSourceDraft">继续编辑</button>
          <button type="button" class="button danger" @click="abandonNewSource">放弃修改</button>
        </div>
      </article>
    </div>

    <section v-if="login.sourceLoginTarget" class="modal-backdrop" @click.self="!login.sourceLoginBusy && login.closeSourceLogin()">
      <article class="app-modal source-edit-modal">
        <button class="detail-close" :disabled="login.sourceLoginBusy" @click="login.closeSourceLogin"><PrototypeIcon name="close"/></button>
        <h2 id="source-login-title">「{{ login.sourceLoginTarget.name }}」书源登录</h2>
        <form v-if="login.sourceLoginForm?.mode === 'form'" @submit.prevent="login.executeSelectedSourceLogin()" class="source-login-form">
          <label v-for="field in login.sourceLoginForm.fields" :key="field.name" class="form-field">
            <span>{{ field.label }}</span>
            <select v-if="field.type === 'select'" v-model="login.sourceLoginCredentials[field.name]" :disabled="login.sourceLoginBusy">
              <option v-for="choice in field.choices ?? []" :key="choice" :value="choice">{{ choice }}</option>
            </select>
            <input v-else-if="field.type === 'toggle'" type="checkbox" :checked="login.sourceLoginCredentials[field.name] === 'true'" :disabled="login.sourceLoginBusy" @change="login.sourceLoginCredentials[field.name] = ($event.target as HTMLInputElement).checked ? 'true' : 'false'" />
            <input v-else v-model="login.sourceLoginCredentials[field.name]" :type="field.password || field.type === 'password' ? 'password' : 'text'" :autocomplete="field.password || field.type === 'password' ? 'new-password' : 'off'" maxlength="4096" :disabled="login.sourceLoginBusy" />
          </label>
          <small>输入交由书源引擎处理并保存在私有登录数据中；密码不会回填，留空时使用该字段已保存的密码。</small>
          <div class="modal-actions">
            <button v-if="login.sourceLoginForm.canLogin" type="submit" class="button primary" :disabled="login.sourceLoginBusy || recoveryRequired">执行登录</button>
            <button v-for="action in login.sourceLoginForm.actions" :key="action.id" type="button" class="button secondary" :disabled="login.sourceLoginBusy || recoveryRequired" @click="login.executeSelectedSourceLogin(action.id)">{{ action.label }}</button>
          </div>
        </form>
        <div v-else-if="login.sourceLoginForm?.mode === 'web'" class="source-login-form">
          <p>{{ login.sourceLoginForm.message }}</p>
          <p v-if="login.sourceLoginWebSessionId" class="modal-description">登录页面已在私有浏览器中打开。完成网页登录后关闭私有浏览器，回此处点“完成并保存 Cookie”。应用只保存 Cookie，不会验证站点是否已认证。</p>
          <div class="modal-actions">
            <button v-if="!login.sourceLoginWebSessionId" type="button" class="button primary" :disabled="login.sourceLoginBusy || recoveryRequired" @click="login.startSelectedSourceWebLogin">{{ login.sourceLoginBusy ? '正在打开…' : '打开网页登录' }}</button>
            <template v-else>
              <button type="button" class="button primary" :disabled="login.sourceLoginBusy || recoveryRequired" @click="login.finishSelectedSourceWebLogin">{{ login.sourceLoginBusy ? '正在保存…' : '完成并保存 Cookie' }}</button>
              <button type="button" class="button secondary" :disabled="login.sourceLoginBusy" @click="login.cancelSelectedSourceWebLogin">取消网页登录</button>
            </template>
          </div>
        </div>
        <p v-else-if="login.sourceLoginForm?.mode === 'webUnavailable'" class="modal-description">{{ login.sourceLoginForm.message }}</p>
        <p v-if="login.sourceLoginFeedback">{{ login.sourceLoginFeedback }}</p>
        <p v-if="login.sourceLoginFormError" class="error-message">{{ login.sourceLoginFormError }}</p>
        <button v-if="!login.sourceLoginForm" class="button secondary small" :disabled="login.sourceLoginBusy || recoveryRequired" @click="login.loadSelectedSourceLoginForm">读取登录表单</button>
        <p class="modal-description">清除保存的登录信息和该域名共享的 Cookie。同域名的其他书源也可能需要重新登录。</p>
        <p>{{ login.sourceLoginBusy ? '正在处理…' : login.sourceHasLoginState === true ? '存在已保存的登录数据' : login.sourceHasLoginState === false ? '未发现当前书源入口可用的登录数据' : '登录状态尚未确认' }}</p>
        <p v-if="login.sourceLoginError" class="error-message">{{ login.sourceLoginError }}</p>
        <div class="modal-actions">
          <button class="button secondary" :disabled="login.sourceLoginBusy || recoveryRequired" @click="login.refreshSelectedSourceLogin">重新读取</button>
          <button class="button danger-outline" :disabled="login.sourceLoginBusy || !!login.sourceLoginWebSessionId || login.sourceHasLoginState === null || recoveryRequired" @click="login.clearSelectedSourceLogin">清除登录状态</button>
          <button class="button secondary" :disabled="login.sourceLoginBusy" @click="login.closeSourceLogin">关闭</button>
        </div>
      </article>
    </section>

    <SourceDebugPanel
      v-if="debug.sourceDebugTarget"
      :target="debug.sourceDebugTarget"
      :sources="management.bookSourceCandidates"
      :keyword="debug.sourceDebugKeyword"
      :busy="debug.sourceDebugBusy"
      :search-result-batch-busy="searchResultBatchBusy"
      :recovery-required="recoveryRequired"
      :stage="debug.sourceDebugStage"
      :error="debug.sourceDebugError"
      :search-errors="debug.sourceDebugErrors"
      :results="debug.sourceDebugResults"
      :searched="debug.sourceDebugSearched"
      :selected-result="debug.sourceDebugSelectedResult"
      :book="debug.sourceDebugBook"
      :chapters="debug.sourceDebugChapters"
      :selected-chapter-index="debug.sourceDebugSelectedChapterIndex"
      :chapter-resource="debug.sourceDebugChapterResource"
      :chapter-text="debug.sourceDebugChapterText"
      @close="debug.closeSourceDebug"
      @update:keyword="debug.sourceDebugKeyword = $event"
      @select-source="debug.selectSourceDebugSource"
      @search="debug.startSourceDebugSearch"
      @inspect-result="debug.inspectSourceDebugResult"
      @preview-chapter="debug.previewSourceDebugChapter"
    />



    <section v-if="management.pendingSourceRemoval.length" class="modal-backdrop" @click.self="!management.sourceRemovalBusy && (management.pendingSourceRemoval = [])"><article class="app-modal confirm-modal"><div class="confirm-symbol source-confirm"><PrototypeIcon name="trash"/></div><h2>移除这些书源？</h2><p>已添加到书架的书籍不会受到影响。</p><div class="modal-actions"><button class="button secondary" :disabled="management.sourceRemovalBusy" @click="management.pendingSourceRemoval = []">取消</button><button class="button danger" :disabled="management.sourceRemovalBusy || recoveryRequired" @click="management.deleteSelectedSources">{{ management.sourceRemovalBusy ? '移除中…' : '确认移除' }}</button></div></article></section>
  </template>
</template>
<style scoped>
.source-data-state{display:grid;min-height:100%;place-content:center;justify-items:center;gap:10px;padding:24px;text-align:center}
.source-data-state h2,.source-data-state p{margin:0}
.source-data-state p{color:var(--app-muted)}
.source-load-warning{margin:0;padding:8px 18px;color:#9e5d2b;background:#fff4e8;font-size:12px}
.source-workspace-frame{min-width:0;width:100%}
.source-workspace-frame:not(.source-workspace-frame--editing){display:flex;flex:1 1 auto;flex-direction:column;min-height:0;height:100%;overflow:hidden}
.source-workspace-frame--editing{width:100%;min-width:0}
.source-workspace-frame :deep(.source-edit-screen){min-height:0;overflow:auto}
@media(min-width:861px){
 .source-workspace-frame--editing{display:flex;flex:1 1 auto;flex-direction:column;min-height:0;height:100%;overflow:hidden}
 .source-workspace-frame :deep(.source-edit-bar){display:none}
 .source-workspace-frame :deep(.source-edit-screen){padding:28px 34px 48px}
}
@media(max-width:860px){.source-workspace-frame--editing :deep(.source-edit-screen){overflow:visible}}
.source-leave-guard{z-index:120}
.source-confirm{color:#9d7952;background:#f7f0e6}
.source-confirm svg{width:22px;height:22px}
.detail-close svg{width:18px;height:18px}
.source-import-modal{width:min(515px,100%);box-sizing:border-box;padding:25px}
.source-import-modal--url{width:min(420px,100%);padding:22px}
.source-import-modal--new{max-height:90dvh;overflow-y:auto}
.source-import-heading{display:flex;align-items:center;justify-content:space-between;gap:10px}
.source-import-heading h2{margin:0;font-size:21px;color:#343544}
.source-import-close{display:grid;place-items:center;flex:none;width:33px;height:33px;border:0;border-radius:9px;background:#f4f4f7;color:#7b7e89;cursor:pointer}
.source-import-close svg{width:18px;height:18px}
.source-import-close:disabled{opacity:.45;cursor:default}
.source-import-url{display:grid;gap:9px;margin-top:10px}
.source-import-new{display:grid;gap:11px;margin-top:22px}
.source-import-modal--url .source-import-heading h2{font-size:17px}
.source-import-url label,.source-import-new label{font-size:12px;font-weight:650;color:#4c4e5c}
.source-import-new textarea{box-sizing:border-box;min-width:0;width:100%;min-height:205px;max-height:42dvh;resize:vertical;padding:12px 13px;border:1px solid #e0e2eb;border-radius:9px;background:#fff;color:#393846;font:12px/1.55 ui-monospace,SFMono-Regular,Consolas,monospace}
.source-import-modal--new{width:min(820px,100%)}
.source-import-new-tabs{display:flex;gap:7px;border-bottom:1px solid #e6e7ee}
.source-import-new-tabs>button{padding:9px 14px 11px;border:0;border-bottom:2px solid transparent;background:transparent;color:#777d8c;font-size:12px;cursor:pointer}
.source-import-new-tabs>button.active{color:var(--app-accent);border-bottom-color:var(--app-accent);font-weight:700}
.source-import-new-tabs>button:disabled{opacity:.55;cursor:default}
.source-import-new>label{display:grid;gap:9px}
.source-import-new :deep(.source-visual-section){padding:15px}
.source-new-discard-mask{z-index:150}
.source-import-hint{margin:0;color:#9194a0;font-size:11px;line-height:1.65}
.source-import-url input{box-sizing:border-box;width:100%;min-height:39px;padding:9px 11px;border:1px solid #dfe1e9;border-radius:9px;background:#fff;color:#323643;font:inherit;font-size:12px}
.source-import-url input.is-invalid{border-color:#c76b76}
.source-import-url>small{font-size:11px;color:#9396a1;line-height:1.5}
.source-import-url>button{justify-self:start}
.source-import-error{margin:0;color:#b55261;font-size:12px}
.source-import-actions{display:flex;justify-content:flex-end;align-items:center;flex-wrap:wrap;gap:10px;padding-top:12px}
.source-import-url .source-import-actions{padding-top:5px}
.source-import-loading{margin:12px 0 0;font-size:12px;color:#6c5fbb}
@media(max-width:550px){.source-import-modal{padding:19px}.source-import-modal--url{padding:22px}}
.source-new-discard-mask .app-modal{width:min(430px,100%);padding:22px;border-radius:14px}
</style>

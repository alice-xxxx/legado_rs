<script setup lang="ts">
import SourceDebugPanel from "./SourceDebugPanel.vue";
import SourceManagerPage from "./SourceManagerPage.vue";
import SourceMetadataEditorPage from "./SourceMetadataEditorPage.vue";
import SourceFormEditor from "./SourceFormEditor.vue";
import SourceLoginFormView from "./SourceLoginForm.vue";
import { useSourceLoginActions } from "./useSourceLoginActions";
import { useSourceDebugActions } from "./useSourceDebugActions";
import { useSourceManagementActions } from "./useSourceManagementActions";
import type { AppBootstrap, SourceMetadata, SourceLoginForm } from "../../api/types";
import { computed, onBeforeUnmount, onMounted, proxyRefs, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { sources, enabledSources, bookSourceCandidates } from "./sourcesStore";
import { tasks as taskList, tasksLoaded as taskListLoaded, tasksError, ensureTasksLoaded } from "../tasks/tasksStore";
import { notify } from "../../app/notifications";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import { onBeforeRouteLeave, useRoute } from "vue-router";
import router, { routeNames } from "../../router";



const errorText = (error: unknown): string => error instanceof Error ? error.message : String(error);

const selectedSourceIds = ref<string[]>(bookSourceCandidates.value.map(source => source.id));
const selectedSourceRows = ref<string[]>([]);
const pendingSourceRemoval = ref<string[]>([]);
const editingSource = ref<SourceMetadata | null>(null);
const sourceBatchBusy = ref(false);
const sourceExportBusy = ref(false);
const sourceImportBusy = ref(false);
const sourceImportOpen = ref(false);
const sourceImportUrl = ref("");
const recoveryRequired = shelfBatchRecoveryRequired;
const searchBusy = computed(() => !taskListLoaded.value || !!tasksError.value || taskList.value.some(task => task.kind === "search"
  && ["queued", "running", "pausing", "cancelling"].includes(task.status)));
const searchResultBatchBusy = ref(false);
let restoreFrontendRevision = 0;
let unlisteners: UnlistenFn[] = [];
let pageDisposed = false;

watch(sources, () => {
  selectedSourceIds.value = selectedSourceIds.value.filter(id => bookSourceCandidates.value.some(source => source.id === id));
  selectedSourceRows.value = selectedSourceRows.value.filter(id => sources.value.some(source => source.id === id));
});

const sourceManagementActions = useSourceManagementActions({
  selectedSourceIds,
  selectedSourceRows,
  sourceBatchBusy,
  sourceExportBusy,
  sourceImportOpen,
  sourceImportBusy,
  sourceImportUrl,
  pendingSourceRemoval,
  editingSource,
  getRestoreFrontendRevision: () => restoreFrontendRevision,
  errorText,
  notify,
});
const sourceDebugActions = useSourceDebugActions({
  selectedSourceRows,
  sourceBatchBusy,
  sourceExportBusy,
  searchBusy,
  searchResultBatchBusy,
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
  selectedSourceRows,
  sourceBatchBusy,
  sourceExportBusy,
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

function reportTaskRefreshError(error: unknown): void {
  if (!pageDisposed) notify(`浠诲姟鐘舵€佸埛鏂板け璐ワ細${errorText(error)}`, "error");
}

const management = proxyRefs({
  ...sourceManagementActions,
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

async function handleAppStateUpdated(): Promise<void> {
  const restoreRevision = ++restoreFrontendRevision;
  selectedSourceIds.value = [];
  selectedSourceRows.value = [];
  pendingSourceRemoval.value = [];
  editingSource.value = null;
  sourceDebugActions.closeSourceDebug();
  sourceLoginActions.closeSourceLogin();
  await ensureTasksLoaded();
  if (pageDisposed || restoreRevision !== restoreFrontendRevision) return;
  selectedSourceIds.value = bookSourceCandidates.value.map(source => source.id);
}

onMounted(async () => {
  try {
    const unlistenAppState = await listen<AppBootstrap>("app-state-updated", () => {
      void handleAppStateUpdated().catch(reportTaskRefreshError);
    });
    if (pageDisposed) unlistenAppState();
    else unlisteners.push(unlistenAppState);
  } catch (error) {
    reportTaskRefreshError(error);
  }
  if (pageDisposed) return;
  await Promise.all([
    ensureTasksLoaded().catch(error => {
      if (!pageDisposed) notify(`浠诲姟鐘舵€佽鍙栧け璐ワ細${errorText(error)}`, "error");
    }),
  ]);
});

onBeforeUnmount(() => {
  pageDisposed = true;
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
const newSourceTemplate = "{}";
const newSourceJson = ref(newSourceTemplate);
const newSourceForm = ref<InstanceType<typeof SourceFormEditor> | null>(null);
const newSourceFormPending = ref(false);
const newSourceDiscardOpen = ref(false);
const hasNewSourceDraft = computed(() => newSourceFormPending.value || newSourceJson.value !== newSourceTemplate);
let pendingSourceImportNavigation: ((allow: boolean) => void) | null = null;
const newSourceError = ref("");
async function submitNewSource(): Promise<void> {
  if (importLocked.value) return;
  const json = newSourceForm.value?.serialize();
  if (!json) return;
  try {
    const parsed: unknown = JSON.parse(json);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) throw new Error("配置必须是对象");
    const source = parsed as Record<string, unknown>;
    if (typeof source.bookSourceName !== "string" || !source.bookSourceName.trim()) throw new Error("请填写来源名称");
    if (typeof source.bookSourceUrl !== "string" || !source.bookSourceUrl.trim()) throw new Error("请填写来源地址");
  } catch (error) {
    newSourceError.value = errorText(error);
    return;
  }
  newSourceError.value = "";
  newSourceJson.value = json;
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
  newSourceError.value = "";
  newSourceForm.value?.reset(newSourceTemplate);
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
        <p class="modal-description">当前书源有尚未保存的修改。{{ pendingSourceSwitch ? '切换来源' : '离开页面' }}会丢弃全部草稿，包括正在编辑的字段。</p>
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
          <SourceFormEditor ref="newSourceForm" :json-text="newSourceJson" :enabled="true"
            :disabled="importLocked" @pending="newSourceFormPending = $event" />
          <p v-if="newSourceError" class="source-import-error">{{ newSourceError }}</p>
          <p class="source-import-hint">与现有来源地址相同时将更新已有配置，而不是生成重复来源。</p>
          <div class="source-import-actions">
            <button type="button" class="button secondary" :disabled="importLocked" @click="closeSourceImport">取消</button>
            <button type="button" class="button primary"
              :disabled="importLocked" @click="submitNewSource">
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

    <section v-if="login.sourceLoginTarget" class="modal-backdrop" @click.self="!login.sourceLoginBusy && login.closeSourceLogin()"
      @keydown.esc.stop.prevent="!login.sourceLoginBusy && login.closeSourceLogin()">
      <article class="app-modal source-login-modal" role="dialog" aria-modal="true" aria-labelledby="source-login-title">
        <header class="source-login-heading">
          <div><h2 id="source-login-title">书源登录</h2><p :title="login.sourceLoginTarget.name">{{ login.sourceLoginTarget.name }}</p></div>
          <button type="button" :disabled="login.sourceLoginBusy" aria-label="关闭登录" @click="login.closeSourceLogin"><PrototypeIcon name="close"/></button>
        </header>
        <div class="source-login-body">
        <SourceLoginFormView v-if="login.sourceLoginForm?.mode === 'form'"
          :key="login.sourceLoginTarget.id" :form="login.sourceLoginForm"
          v-model="login.sourceLoginCredentials" :busy="login.sourceLoginBusy"
          :disabled="login.sourceLoginBusy || recoveryRequired" @execute="login.executeSelectedSourceLogin" />
        <div v-else-if="login.sourceLoginForm?.mode === 'web'" class="source-login-form">
          <p v-if="login.sourceLoginWebSessionId" class="source-login-web-hint">在网页中登录，关闭窗口后点「完成」。</p>
          <p v-else-if="!login.sourceLoginFeedback" class="source-login-web-hint">打开网页完成登录。</p>
          <div class="source-login-web-actions">
            <button v-if="!login.sourceLoginWebSessionId" type="button" class="button primary" :disabled="login.sourceLoginBusy || recoveryRequired" @click="login.startSelectedSourceWebLogin">{{ login.sourceLoginBusy ? '正在打开…' : login.sourceLoginFeedback ? '重新登录' : '打开登录页' }}</button>
            <template v-else>
              <button type="button" class="button primary" :disabled="login.sourceLoginBusy || recoveryRequired" @click="login.finishSelectedSourceWebLogin">{{ login.sourceLoginBusy ? '正在保存…' : '完成' }}</button>
              <button type="button" class="button secondary" :disabled="login.sourceLoginBusy" @click="login.cancelSelectedSourceWebLogin">取消</button>
            </template>
          </div>
        </div>
        <p v-else-if="login.sourceLoginForm?.mode === 'webUnavailable'" class="modal-description">{{ login.sourceLoginForm.message }}</p>
        <p v-if="login.sourceLoginFeedback && !login.sourceLoginWebSessionId" class="source-login-feedback" role="status">{{ login.sourceLoginFeedback }}</p>
        <p v-if="login.sourceLoginFormError" class="source-login-error" role="alert">{{ login.sourceLoginFormError }}</p>
        <button v-if="!login.sourceLoginForm" class="button secondary small" :disabled="login.sourceLoginBusy || recoveryRequired" @click="login.loadSelectedSourceLoginForm">读取登录表单</button>
        <div class="source-login-state">
          <span :class="{ saved: login.sourceHasLoginState === true }"></span>
          <p>{{ login.sourceLoginBusy ? '正在处理…' : login.sourceHasLoginState === true ? '已保存登录数据' : login.sourceHasLoginState === false ? '暂无登录数据' : '登录状态尚未确认' }}</p>
          <button type="button" :disabled="login.sourceLoginBusy || recoveryRequired" @click="login.refreshSelectedSourceLogin">刷新</button>
        </div>
        <p v-if="login.sourceLoginError" class="source-login-error" role="alert">{{ login.sourceLoginError }}</p>
        <details class="source-login-storage">
          <summary>登录数据</summary>
          <button type="button" class="button danger-outline" :disabled="login.sourceLoginBusy || !!login.sourceLoginWebSessionId || login.sourceHasLoginState === null || recoveryRequired" @click="login.clearSelectedSourceLogin">清除登录数据</button>
        </details>
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
<style scoped>
.source-workspace-frame{display:flex;flex:1 1 auto;flex-direction:column;min-width:0;min-height:0;width:100%;height:100%;overflow:hidden}
.source-workspace-frame :deep(.source-edit-screen){min-height:0;overflow:hidden}
@media(min-width:861px){
 .source-workspace-frame :deep(.source-edit-screen){--source-editor-inset:24px}
}
.source-leave-guard{z-index:150}
.source-login-modal{display:flex;flex-direction:column;width:min(520px,100%);max-height:min(88dvh,800px);padding:0;overflow:hidden;border-radius:14px}
.source-login-heading{display:flex;flex:none;align-items:center;justify-content:space-between;gap:14px;padding:22px 24px 18px;border-bottom:1px solid var(--app-line)}
.source-login-heading>div{min-width:0}
.source-login-heading h2{margin:0;color:var(--app-text);font-size:var(--ui-font-title);font-weight:650}
.source-login-heading p{margin:6px 0 0;color:var(--app-muted);font-size:var(--ui-font-caption);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.source-login-heading>button{display:grid;place-items:center;flex:none;width:38px;height:38px;border-radius:8px;background:var(--app-panel-soft);color:var(--app-muted);cursor:pointer}
.source-login-heading svg{width:18px;height:18px}
.source-login-body{min-height:0;padding:22px 24px;overflow-y:auto;overscroll-behavior:contain}
.source-login-web-hint{margin:0;color:var(--app-muted);font-size:var(--ui-font-caption);line-height:1.6}
.source-login-web-actions{display:flex;gap:10px;margin-top:18px}
.source-login-web-actions>.primary{flex:1}
.source-login-web-actions>.button{min-height:44px}
.source-login-feedback,.source-login-error{margin:16px 0 0;padding:11px 13px;border-radius:8px;font-size:var(--ui-font-caption);line-height:1.6;overflow-wrap:anywhere;white-space:pre-wrap}
.source-login-feedback{background:var(--app-accent-soft);color:var(--app-accent-ink)}
.source-login-error{background:var(--app-danger-soft);color:var(--app-danger)}
.source-login-state{display:flex;align-items:center;gap:8px;margin-top:22px;padding-top:18px;border-top:1px solid var(--app-line)}
.source-login-state>span{flex:none;width:7px;height:7px;border-radius:50%;background:var(--app-accent-soft)}
.source-login-state>span.saved{background:var(--app-success)}
.source-login-state>p{flex:1;margin:0;font-size:var(--ui-font-caption);color:var(--app-muted)}
.source-login-state>button{min-height:36px;padding:0 8px;color:var(--app-accent);font-size:var(--ui-font-body);cursor:pointer}
.source-login-storage{margin-top:12px;color:var(--app-subtle);font-size:var(--ui-font-caption)}
.source-login-storage>summary{width:max-content;padding:8px 0;cursor:pointer}
.source-login-storage>p{margin:7px 0 12px;line-height:1.6}
@media(max-width:550px){.source-login-heading{padding:19px 18px 16px}.source-login-body{padding:20px 18px}.source-login-heading h2{font-size:var(--ui-font-heading)}}
.source-confirm{color:var(--app-warning);background:var(--app-panel-soft)}
.source-confirm svg{width:22px;height:22px}
.detail-close svg{width:18px;height:18px}
.source-import-modal{width:min(515px,100%);box-sizing:border-box;padding:25px}
.source-import-modal--url{width:min(420px,100%);padding:22px}
.source-import-modal--new{max-height:90dvh;overflow-y:auto}
.source-import-heading{display:flex;align-items:center;justify-content:space-between;gap:10px}
.source-import-heading h2{margin:0;font-size:var(--ui-font-title);color:var(--app-text)}
.source-import-close{display:grid;place-items:center;flex:none;width:33px;height:33px;border:0;border-radius:9px;background:var(--app-panel-soft);color:var(--app-muted);cursor:pointer}
.source-import-close svg{width:18px;height:18px}
.source-import-close:disabled{opacity:.45;cursor:default}
.source-import-url{display:grid;gap:9px;margin-top:10px}
.source-import-new{display:grid;gap:11px;margin-top:22px}
.source-import-modal--url .source-import-heading h2{font-size:var(--ui-font-heading)}
.source-import-url label,.source-import-new label{font-size:var(--ui-font-body);font-weight:650;color:var(--app-text)}
.source-import-modal--new{width:min(820px,100%)}
.source-new-discard-mask{z-index:150}
.source-import-hint{margin:0;color:var(--app-subtle);font-size:var(--ui-font-caption);line-height:1.65}
.source-import-url input{box-sizing:border-box;width:100%;min-height:39px;padding:9px 11px;border:1px solid var(--app-line);border-radius:9px;background:var(--app-surface);color:var(--app-text);font:inherit;font-size:var(--ui-font-body)}
.source-import-url input.is-invalid{border-color:var(--app-danger-line)}
.source-import-url>small{font-size:var(--ui-font-caption);color:var(--app-subtle);line-height:1.5}
.source-import-url>button{justify-self:start}
.source-import-error{margin:0;color:var(--app-danger);font-size:var(--ui-font-caption)}
.source-import-actions{display:flex;justify-content:flex-end;align-items:center;flex-wrap:wrap;gap:10px;padding-top:12px}
.source-import-url .source-import-actions{padding-top:5px}
.source-import-loading{margin:12px 0 0;font-size:var(--ui-font-caption);color:var(--app-accent-ink)}
@media(max-width:550px){.source-import-modal{padding:19px}.source-import-modal--url{padding:22px}}
.source-new-discard-mask .app-modal{width:min(430px,100%);padding:22px;border-radius:14px}
</style>

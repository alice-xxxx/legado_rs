<script setup lang="ts">
import SourceDebugPanel from "./SourceDebugPanel.vue";
import SourceManagerPage from "./SourceManagerPage.vue";
import SourceMetadataEditorPage from "./SourceMetadataEditorPage.vue";
import SourceLoginFormView from "./SourceLoginForm.vue";
import { useSourceLoginActions } from "./useSourceLoginActions";
import { useSourceDebugActions } from "./useSourceDebugActions";
import { useSourceManagementActions } from "./useSourceManagementActions";
import type { AppBootstrap, SourceMetadata, SourceLoginForm } from "../../api/types";
import { computed, onBeforeUnmount, onMounted, proxyRefs, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { sources, enabledSources, bookSourceCandidates } from "./sourcesStore";
import { searchBusy } from "../discovery/searchState";
import { notify } from "../../app/notifications";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import { onBeforeRouteLeave, onBeforeRouteUpdate, useRoute } from "vue-router";
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
const sourceLoginForm = ref<SourceLoginForm | null>(null);
const sourceLoginWebOpen = ref(false);
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
  sourceLoginForm,
  sourceLoginWebOpen,
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

function reportSourceLoadError(error: unknown): void {
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
  sourceLoginForm,
  sourceLoginWebOpen,
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
  sourceManagementActions.sourceEditCreating.value = false;
  pendingDraft.value = false;
  leaveEditorRoute();
  sourceDebugActions.closeSourceDebug();
  sourceLoginActions.closeSourceLogin();
  if (pageDisposed || restoreRevision !== restoreFrontendRevision) return;
  selectedSourceIds.value = bookSourceCandidates.value.map(source => source.id);
}

onMounted(async () => {
  try {
    const unlistenAppState = await listen<AppBootstrap>("app-state-updated", () => {
      void handleAppStateUpdated().catch(reportSourceLoadError);
    });
    if (pageDisposed) unlistenAppState();
    else unlisteners.push(unlistenAppState);
  } catch (error) {
    reportSourceLoadError(error);
  }
  if (pageDisposed) return;

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

const pendingDraft = ref(false);
const confirmRouteLeave = ref(false);
const pendingSourceSwitch = ref<SourceMetadata | "new" | null>(null);
let initialDesktopSourceOpened = false;
const currentRoute = useRoute();
const routeSourceImportMode = computed(() => currentRoute.name === routeNames.sources
  && (currentRoute.query.sourceImport === "url" || currentRoute.query.sourceImport === "new")
  ? currentRoute.query.sourceImport : "");
const routeSourceEdit = computed(() => currentRoute.name === routeNames.sources
  && typeof currentRoute.query.sourceEdit === "string" ? currentRoute.query.sourceEdit : "");
let editorNavigationPending = false;
function sourceListLocation() {
  const query = { ...currentRoute.query };
  delete query.sourceEdit;
  delete query.sourceImport;
  delete query.importUrl;
  return { name: routeNames.sources, query };
}
function leaveEditorRoute(): void {
  if (!routeSourceEdit.value) return;
  const list = sourceListLocation();
  if (router.options.history.state.back === router.resolve(list).fullPath) router.back();
  else void router.replace(list);
}
function closeSourceEditor(): void {
  if (sourceManagementActions.sourceEditBusy.value) return;
  sourceManagementActions.closeSourceEdit();
  pendingDraft.value = false;
  leaveEditorRoute();
}
async function saveSourceEditor(): Promise<void> {
  await sourceManagementActions.saveSourceDefinitionJson();
  if (!sourceManagementActions.sourceEditorOpen.value) {
    pendingDraft.value = false;
    leaveEditorRoute();
  }
}
let suppressSourceImportQuerySync = false;
function syncSourceImportRoute(mode: "new" | "url" | null): void {
  if (suppressSourceImportQuerySync || currentRoute.name !== routeNames.sources) return;
  const query = { ...currentRoute.query };
  if (mode) {
    query.sourceImport = mode;
    if (mode === "new") delete query.importUrl;
  } else {
    delete query.sourceImport;
    delete query.importUrl;
  }
  if (query.sourceImport === currentRoute.query.sourceImport && query.importUrl === currentRoute.query.importUrl) return;
  void router.replace({ name: routeNames.sources, query });
}
watch(sourceImportOpen, opened => {
  if (opened) syncSourceImportRoute("url");
  else if (routeSourceImportMode.value === "url") syncSourceImportRoute(null);
}, { flush: "sync" });
watch(sourceManagementActions.sourceEditCreating, creating => {
  if (routeSourceEdit.value) return;
  if (creating) syncSourceImportRoute("new");
  else if (routeSourceImportMode.value === "new") syncSourceImportRoute(null);
}, { flush: "sync" });
watch(routeSourceImportMode, mode => {
  if (mode === "new") {
    sourceImportOpen.value = false;
    if (!sourceManagementActions.sourceEditCreating.value) sourceManagementActions.beginSourceCreate();
  } else if (mode === "url") {
    const url = currentRoute.query.importUrl;
    sourceImportUrl.value = typeof url === "string" ? url : "";
    sourceImportOpen.value = true;
  } else if (sourceImportOpen.value) {
    suppressSourceImportQuerySync = true;
    sourceImportOpen.value = false;
    suppressSourceImportQuerySync = false;
  }
}, { immediate: true });
watch(routeSourceEdit, key => {
  if (!key || pageDisposed || currentRoute.name !== routeNames.sources) return;
  if (key === "new") {
    if (!sourceManagementActions.sourceEditCreating.value) sourceManagementActions.beginSourceCreate();
    return;
  }
  if (editingSource.value?.id === key || sourceManagementActions.sourceEditBusy.value) return;
  const source = sources.value.find(entry => entry.id === key);
  if (source) void sourceManagementActions.beginSourceEdit(source).then(() => {
    if (pageDisposed || routeSourceEdit.value !== key) sourceManagementActions.closeSourceEdit();
    else if (!sourceManagementActions.sourceEditorOpen.value) leaveEditorRoute();
  });
  else {
    notify("该源不存在或已被移除。", "error");
    leaveEditorRoute();
  }
}, { immediate: true });
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
function openSourceImport(): void {
  if (importLocked.value) return;
  sourceImportUrl.value = "";
  sourceImportOpen.value = true;
}
function closeSourceImport(): void {
  if (importLocked.value) return;
  sourceImportOpen.value = false;
}
function submitSourceImport(): void {
  if (importLocked.value || !sourceImportUrl.value.trim() || urlImportError.value) return;
  void sourceManagementActions.importSourceFromUrl();
}
watch(() => sources.value.length, count => {
  if (initialDesktopSourceOpened || !count || sourceManagementActions.sourceEditorOpen.value
      || sourceImportOpen.value || typeof window === "undefined"
      || window.innerWidth <= 860 || routeSourceEdit.value || routeSourceImportMode.value) return;
  initialDesktopSourceOpened = true;
  const firstSource = sources.value[0];
  if (firstSource) void sourceManagementActions.beginSourceEdit(firstSource);
}, { immediate: true });
function requestEditorSwitch(source?: SourceMetadata): void {
  if (editorNavigationPending || sourceManagementActions.sourceEditBusy.value || recoveryRequired.value) return;
  initialDesktopSourceOpened = true;
  if (!sourceManagementActions.sourceEditorOpen.value) {
    if (typeof window !== "undefined" && window.innerWidth <= 860) {
      const query = { ...currentRoute.query };
      query.sourceEdit = source?.id ?? "new";
      delete query.sourceImport;
      delete query.importUrl;
      editorNavigationPending = true;
      const navigation = { name: routeNames.sources, query };
      void (routeSourceEdit.value ? router.replace(navigation) : router.push(navigation))
        .catch(error => notify(`打开源编辑失败：${errorText(error)}`, "error"))
        .finally(() => { editorNavigationPending = false; });
      return;
    }
    if (source) void sourceManagementActions.beginSourceEdit(source);
    else sourceManagementActions.beginSourceCreate();
    return;
  }
  if (source ? editingSource.value?.id === source.id : sourceManagementActions.sourceEditCreating.value) return;
  if (isEdited()) {
    pendingSourceSwitch.value = source ?? "new";
    confirmRouteLeave.value = true;
    return;
  }
  sourceManagementActions.closeSourceEdit();
  if (routeSourceEdit.value) {
    void router.replace({ name: routeNames.sources, query: { ...currentRoute.query, sourceEdit: source?.id ?? "new" } });
    return;
  }
  if (source) void sourceManagementActions.beginSourceEdit(source);
  else sourceManagementActions.beginSourceCreate();
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
    if (resolve) suppressSourceImportQuerySync = true;
    sourceManagementActions.closeSourceEdit();
    pendingDraft.value = false;
    suppressSourceImportQuerySync = false;
    if (target) requestEditorSwitch(target === "new" ? undefined : target);
  } else if (target === "new" && !sourceManagementActions.sourceEditCreating.value) {
    syncSourceImportRoute(null);
  }
  resolve?.(allow);
}
function guardEditorNavigation(): boolean | Promise<boolean> {
  if (sourceManagementActions.sourceEditBusy.value) return false;
  if (!sourceManagementActions.sourceEditorOpen.value) return true;
  if (!isEdited()) {
    suppressSourceImportQuerySync = true;
    sourceManagementActions.closeSourceEdit();
    suppressSourceImportQuerySync = false;
    return true;
  }
  // Router back/side navigation cannot bypass the editor's own Cancel dialog.
  return new Promise<boolean>(resolve => {
    pendingSourceSwitch.value = null;
    resolveNavigation?.(false);
    resolveNavigation = resolve;
    confirmRouteLeave.value = true;
  });
}
onBeforeRouteLeave(() => {
  if (sourceImportOpen.value) {
    if (importLocked.value) return false;
    suppressSourceImportQuerySync = true;
    sourceImportOpen.value = false;
    suppressSourceImportQuerySync = false;
  }
  return guardEditorNavigation();
});
onBeforeRouteUpdate(to => {
  if (routeSourceEdit.value && to.query.sourceEdit !== routeSourceEdit.value) return guardEditorNavigation();
  if (sourceManagementActions.sourceEditCreating.value && to.query.sourceImport !== "new") return guardEditorNavigation();
  return true;
});
onBeforeUnmount(() => {
  resolveNavigation?.(false);
});
</script>

<template>
  <div class="source-workspace-frame" :class="{ 'source-workspace-frame--editing': management.sourceEditorOpen }">
  <SourceManagerPage
    :editor-open="management.sourceEditorOpen"
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
    @create-open="requestEditorSwitch()"
    @import-local="management.importSourceFileFromPicker()"
    @import-url-open="openSourceImport"
    @login="login.openSelectedSourceLogin"
    @debug="debug.openSelectedSourceDebug"
    @set-group="management.setSelectedSourcesGroup"
    @set-enabled="management.setSelectedSourcesEnabled"
    @export="management.exportSelectedSources"
    @remove="management.pendingSourceRemoval = $event"
    @edit="requestEditorSwitch"
    @switch-editor="requestEditorSwitch"
    @back="returnFromSources"
  >
    <template #editor>
      <SourceMetadataEditorPage
        v-if="management.sourceEditorOpen"
        :key="management.editingSource?.id ?? 'new'"
        :source="management.editingSource ?? undefined"
        v-model:json-text="management.sourceEditJson"
        :original-json="management.sourceEditOriginalJson"
        :busy="management.sourceEditBusy"
        :locked="importLocked"
        @save="saveSourceEditor"
        @cancel="closeSourceEditor"
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
        <h2>{{ pendingSourceSwitch === 'new' ? '新建来源？' : pendingSourceSwitch ? '切换编辑来源？' : '放弃源修改？' }}</h2>
        <p class="modal-description">当前源有尚未保存的修改。{{ pendingSourceSwitch ? '切换来源' : '离开页面' }}会丢弃全部草稿，包括正在编辑的字段。</p>
        <div class="modal-actions">
          <button type="button" class="button secondary" data-back-action @click="chooseRouteLeave(false)">继续编辑</button>
          <button type="button" class="button danger" @click="chooseRouteLeave(true)">{{ pendingSourceSwitch === 'new' ? '放弃并新建' : pendingSourceSwitch ? '放弃并切换' : '放弃并离开' }}</button>
        </div>
      </article>
    </section>

    <section v-if="management.sourceImportOpen" class="modal-backdrop" @click.self="closeSourceImport">
      <article class="app-modal source-import-modal"
@keydown.esc="closeSourceImport">
        <header class="source-import-heading">
          <h2 id="source-import-title">从网络链接导入</h2>
          <button type="button" class="source-import-close" data-back-action
            :disabled="importLocked" @click="closeSourceImport"><PrototypeIcon name="close"/></button>
        </header>
        <form class="source-import-url" @submit.prevent="submitSourceImport">
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
        <p v-if="management.sourceImportBusy" class="source-import-loading">正在下载并导入来源…</p>
      </article>
    </section>
    <section v-if="login.sourceLoginTarget" class="modal-backdrop" @click.self="login.closeSourceLogin()"
      @keydown.esc.stop.prevent="login.closeSourceLogin()">
      <article class="app-modal source-login-modal" role="dialog" aria-modal="true" aria-labelledby="source-login-title">
        <header class="source-login-heading">
          <div><h2 id="source-login-title">来源登录</h2><p :title="login.sourceLoginTarget.name">{{ login.sourceLoginTarget.name }}</p></div>
          <button type="button" data-back-action aria-label="关闭登录" @click="login.closeSourceLogin"><PrototypeIcon name="close"/></button>
        </header>
        <div class="source-login-body">
        <SourceLoginFormView v-if="login.sourceLoginForm?.mode === 'form'"
          :key="login.sourceLoginTarget.id" :form="login.sourceLoginForm"
          v-model="login.sourceLoginCredentials" :busy="login.sourceLoginBusy"
          :disabled="login.sourceLoginBusy || recoveryRequired" @execute="login.executeSelectedSourceLogin" />
        <div v-else-if="login.sourceLoginForm?.mode === 'web'" class="source-login-form">
          <p v-if="login.sourceLoginWebOpen" class="source-login-web-hint">在网页中登录后，回到这里点「完成」保存。</p>
          <p v-else-if="!login.sourceLoginFeedback" class="source-login-web-hint">打开网页完成登录。</p>
          <div class="source-login-web-actions">
            <button v-if="!login.sourceLoginWebOpen" type="button" class="button primary" :disabled="login.sourceLoginBusy || recoveryRequired" @click="login.runLogin({ type: 'open' })">{{ login.sourceLoginBusy ? '正在打开…' : login.sourceLoginFeedback ? '重新登录' : '打开登录页' }}</button>
            <template v-else>
              <button type="button" class="button primary" :disabled="login.sourceLoginBusy || recoveryRequired" @click="login.runLogin({ type: 'submit' })">{{ login.sourceLoginBusy ? '正在保存…' : '完成' }}</button>
              <button type="button" class="button secondary" @click="login.closeSourceLogin">取消</button>
            </template>
          </div>
        </div>
        <p v-if="login.sourceLoginFeedback && !login.sourceLoginWebOpen" class="source-login-feedback" role="status">{{ login.sourceLoginFeedback }}</p>
        <p v-if="login.sourceLoginForm && login.sourceLoginFormError" class="source-login-error" role="alert">{{ login.sourceLoginFormError }}</p>
        <div v-if="!login.sourceLoginForm && !login.sourceLoginBusy" class="source-login-load-state">
          <p v-if="login.sourceLoginFormError" class="source-login-error" role="alert">{{ login.sourceLoginFormError }}</p>
          <button type="button" class="source-login-retry" :disabled="recoveryRequired" @click="login.runLogin({ type: 'open' })">
            <PrototypeIcon name="refresh" />{{ login.sourceLoginFormError ? '重试' : '读取登录方式' }}
          </button>
        </div>
        <div class="source-login-state">
          <span :class="{ saved: login.sourceHasLoginState === true }"></span>
          <p role="status">{{ login.sourceLoginBusy ? '处理中…' : login.sourceHasLoginState === true ? '已保存登录数据' : login.sourceHasLoginState === false ? '暂无登录数据' : '状态未确认' }}</p>
          <button type="button" aria-label="刷新登录状态" :disabled="login.sourceLoginBusy || recoveryRequired" @click="login.runLogin({ type: 'open' })"><PrototypeIcon name="refresh" />刷新</button>
        </div>
        <details v-if="login.sourceHasLoginState === true" class="source-login-storage">
          <summary>管理登录数据</summary>
          <button type="button" class="button danger-outline" :disabled="login.sourceLoginBusy || !!login.sourceLoginWebOpen || login.sourceHasLoginState === null || recoveryRequired" @click="login.clearSelectedSourceLogin">清除登录数据</button>
        </details>
        </div>
      </article>
    </section>

    <SourceDebugPanel
      v-if="debug.sourceDebugTarget"
      :target="debug.sourceDebugTarget"
      :keyword="debug.sourceDebugKeyword"
      :keyword-loading="debug.sourceDebugKeywordLoading"
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
      @search="debug.startSourceDebugSearch"
      @inspect-result="debug.inspectSourceDebugResult"
      @preview-chapter="debug.previewSourceDebugChapter"
    />



    <section v-if="management.pendingSourceRemoval.length" class="modal-backdrop" @click.self="!management.sourceRemovalBusy && (management.pendingSourceRemoval = [])"><article class="app-modal confirm-modal"><div class="confirm-symbol source-confirm"><PrototypeIcon name="trash"/></div><h2>移除这些源？</h2><p>已添加到书架的书籍不会受到影响。</p><div class="modal-actions"><button class="button secondary" data-back-action :disabled="management.sourceRemovalBusy" @click="management.pendingSourceRemoval = []">取消</button><button class="button danger" :disabled="management.sourceRemovalBusy || recoveryRequired" @click="management.deleteSelectedSources">{{ management.sourceRemovalBusy ? '移除中…' : '确认移除' }}</button></div></article></section>
</template>
<style scoped>
.source-workspace-frame{display:flex;flex:1 1 auto;flex-direction:column;min-width:0;min-height:0;width:100%;height:100%;overflow:hidden}
.source-workspace-frame :deep(.source-edit-screen){min-height:0;overflow:hidden}
@media(min-width:861px){
 .source-workspace-frame :deep(.source-edit-screen){--source-editor-inset:24px}
}
.source-leave-guard{z-index:150}
.source-login-modal{display:flex;flex-direction:column;width:min(520px,100%);min-width:0;max-height:min(88dvh,800px);padding:0;overflow:hidden;border-radius:12px}
.source-login-heading{display:flex;flex:none;align-items:center;justify-content:space-between;gap:12px;padding:12px 20px;border-bottom:1px solid var(--app-line)}
.source-login-heading>div{min-width:0}
.source-login-heading h2{margin:0;color:var(--app-text);font-size:var(--ui-font-heading);font-weight:650}
.source-login-heading p{margin:4px 0 0;color:var(--app-muted);font-size:var(--ui-font-caption);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.source-login-heading>button{display:grid;place-items:center;flex:none;width:36px;height:36px;border-radius:var(--ui-radius);background:transparent;color:var(--app-muted);cursor:pointer}
.source-login-heading svg{width:var(--ui-icon-size);height:var(--ui-icon-size)}
.source-login-body{display:grid;gap:12px;min-width:0;min-height:0;padding:16px 20px 20px;overflow-y:auto;overflow-x:hidden;overscroll-behavior:contain;overflow-wrap:anywhere}
.source-login-note{margin:0;color:var(--app-muted);font-size:var(--ui-font-body);line-height:1.5}
.source-login-load-state{display:flex;align-items:flex-start;gap:12px;min-width:0}
.source-login-load-state>.source-login-error{flex:1;min-width:0}
.source-login-retry{display:inline-flex;align-items:center;gap:6px;flex:none;min-height:var(--ui-compact-control-height);padding:0 6px;border-radius:var(--ui-radius);color:var(--app-muted);font-size:var(--ui-font-caption);cursor:pointer}
.source-login-retry svg,.source-login-state button svg{width:16px;height:16px}
.source-login-web-hint{margin:0;color:var(--app-muted);font-size:var(--ui-font-caption);line-height:1.6}
.source-login-web-actions{display:flex;justify-content:flex-end;flex-wrap:wrap;gap:8px;margin-top:12px}
.source-login-web-actions>.button{min-height:var(--ui-control-height);font-weight:500}
.source-login-feedback,.source-login-error{margin:0;font-size:var(--ui-font-caption);line-height:1.6;overflow-wrap:anywhere;white-space:pre-wrap}
.source-login-feedback{color:var(--app-accent-ink)}
.source-login-error{color:var(--app-danger)}
.source-login-state{display:flex;align-items:center;gap:8px;min-width:0;padding-top:8px;border-top:1px solid var(--app-line)}
.source-login-state>span{flex:none;width:7px;height:7px;border-radius:50%;background:var(--app-accent-soft)}
.source-login-state>span.saved{background:var(--app-success)}
.source-login-state>p{flex:1;min-width:0;margin:0;font-size:var(--ui-font-caption);color:var(--app-muted)}
.source-login-state>button{display:inline-flex;align-items:center;gap:6px;flex:none;min-height:var(--ui-compact-control-height);padding:0 6px;border-radius:var(--ui-radius);color:var(--app-muted);font-size:var(--ui-font-caption);cursor:pointer}
.source-login-storage{color:var(--app-muted);font-size:var(--ui-font-caption)}
.source-login-storage>summary{width:max-content;padding:4px 0;cursor:pointer}
.source-login-storage>.button{min-height:var(--ui-compact-control-height);margin-top:8px;border-color:var(--app-danger-line);background:var(--app-surface);color:var(--app-danger);font-weight:500}
.source-login-heading>button:focus-visible,.source-login-retry:focus-visible,.source-login-state>button:focus-visible,.source-login-storage>summary:focus-visible{outline:2px solid var(--app-accent);outline-offset:2px}
.source-confirm{color:var(--app-warning);background:var(--app-panel-soft)}
.source-confirm svg{width:22px;height:22px}
.detail-close svg{width:18px;height:18px}
.source-import-modal{width:min(420px,100%);box-sizing:border-box;padding:22px}
.source-import-heading{display:flex;align-items:center;justify-content:space-between;gap:10px}
.source-import-heading h2{margin:0;font-size:var(--ui-font-title);color:var(--app-text)}
.source-import-close{display:grid;place-items:center;flex:none;width:33px;height:33px;border:0;border-radius:9px;background:var(--app-panel-soft);color:var(--app-muted);cursor:pointer}
.source-import-close svg{width:18px;height:18px}
.source-import-close:disabled{opacity:.45;cursor:default}
.source-import-url{display:grid;gap:9px;margin-top:10px}
.source-import-modal--url .source-import-heading h2{font-size:var(--ui-font-heading)}
.source-import-url label,.source-import-new label{font-size:var(--ui-font-body);font-weight:650;color:var(--app-text)}
.source-import-modal--new{width:min(820px,100%)}
.source-new-discard-mask{z-index:150}
.source-import-url input{box-sizing:border-box;width:100%;min-height:39px;padding:9px 11px;border:1px solid var(--app-line);border-radius:9px;background:var(--app-surface);color:var(--app-text);font:inherit;font-size:var(--ui-font-body)}
.source-import-url input.is-invalid{border-color:var(--app-danger-line)}
.source-import-url>small{font-size:var(--ui-font-caption);color:var(--app-subtle);line-height:1.5}
.source-import-url>button{justify-self:start}
.source-import-error{margin:0;color:var(--app-danger);font-size:var(--ui-font-caption)}
.source-import-actions{display:flex;justify-content:flex-end;align-items:center;flex-wrap:wrap;gap:10px;padding-top:12px}
.source-import-url .source-import-actions{padding-top:5px}
.source-import-loading{margin:12px 0 0;font-size:var(--ui-font-caption);color:var(--app-accent-ink)}
</style>

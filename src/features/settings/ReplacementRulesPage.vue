<script setup lang="ts">
// This page owns its rule and shelf snapshots, plus their mutations.
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { onBeforeRouteLeave, useRoute } from "vue-router";
import router, { settingsRouteNames } from "../../router";
import { appBootstrap } from "../../api/core";
import { readResource } from "../../api/resources";
import type { AppBootstrap, DisplayReplacementRule, ResourceDescriptor, ShelfResource } from "../../api/types";
import BackButton from "../../ui/BackButton.vue";
import ManagedListRow from "../../ui/ManagedListRow.vue";
import ManagedListToolbar from "../../ui/ManagedListToolbar.vue";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import { useOutsidePointerDismiss } from "../../ui/useOutsidePointerDismiss";
import { useReplacementRules } from "./useReplacementRules";

defineOptions({ inheritAttrs: false });

type RuleStatusFilter = "all" | "enabled" | "disabled";

const route = useRoute();
const shelf = ref<ShelfResource>({ schemaVersion: 1, books: [] });
const restoreRevision = ref(0);
const loadBusy = ref(true);
const loadError = ref("");
const notice = ref("");
const noticeKind = ref<"success" | "error">("success");
let noticeTimer: ReturnType<typeof setTimeout> | undefined;
let shelfReadRevision = 0;
let bootstrapReadRevision = 0;
let disposed = false;
const unlisteners: UnlistenFn[] = [];

function notify(message: string, kind: "success" | "error" = "success"): void {
  notice.value = message;
  noticeKind.value = kind;
  if (noticeTimer) clearTimeout(noticeTimer);
  noticeTimer = setTimeout(() => { notice.value = ""; }, 3200);
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

const replacementRulesFeature = useReplacementRules({
  getRestoreRevision: () => restoreRevision.value,
  notify,
  errorText,
});
const rules = replacementRulesFeature.replacementRules;
const replacementImportOpen = replacementRulesFeature.replacementImportOpen;
const replacementImportUrl = replacementRulesFeature.replacementImportUrl;
const replacementImportError = replacementRulesFeature.replacementImportError;
const replacementImportBusy = replacementRulesFeature.replacementImportBusy;
const busy = computed(() => loadBusy.value || replacementImportBusy.value
  || replacementRulesFeature.replacementExportBusy.value
  || replacementRulesFeature.replacementRefreshBusy.value
  || replacementRulesFeature.replacementDeleteBusy.value
  || replacementRulesFeature.replacementAddBusy.value);
const scopeOptions = computed(() => [
  { value: "all", label: "全部书籍" },
  ...shelf.value.books.map((book) => ({ value: `book:${book.id}`, label: book.title })),
]);

async function refreshShelf(descriptor: ResourceDescriptor): Promise<void> {
  const revision = ++shelfReadRevision;
  const restoreAtStart = restoreRevision.value;
  const document = await readResource<ShelfResource>(descriptor);
  if (disposed || revision !== shelfReadRevision || restoreAtStart !== restoreRevision.value) return;
  shelf.value = { ...document, groups: document.groups ?? [], books: document.books ?? [] };
}

async function loadBootstrap(snapshot?: AppBootstrap): Promise<void> {
  const revision = ++bootstrapReadRevision;
  const restoreAtStart = restoreRevision.value;
  loadBusy.value = true;
  loadError.value = "";
  try {
    const current = snapshot ?? await appBootstrap();
    if (disposed || revision !== bootstrapReadRevision || restoreAtStart !== restoreRevision.value) return;
    await Promise.all([
      refreshShelf(current.shelf),
      replacementRulesFeature.refreshReplacementRules(current.replacementRules),
    ]);
  } catch (error) {
    if (!disposed && revision === bootstrapReadRevision && restoreAtStart === restoreRevision.value) {
      loadError.value = errorText(error);
    }
  } finally {
    if (!disposed && revision === bootstrapReadRevision && restoreAtStart === restoreRevision.value) {
      loadBusy.value = false;
    }
  }
}

async function listenForResourceUpdates(): Promise<void> {
  const unlistenShelf = await listen<ResourceDescriptor>("shelf-updated", (event) => {
    void refreshShelf(event.payload).catch((error) => notify(`读取书架范围失败：${errorText(error)}`, "error"));
  });
  if (disposed) { unlistenShelf(); return; }
  unlisteners.push(unlistenShelf);

  const unlistenResources = await listen<{ kind: string; resource: ResourceDescriptor }>("resource-updated", (event) => {
    if (event.payload.kind !== "replacementRules") return;
    void replacementRulesFeature.refreshReplacementRules(event.payload.resource)
      .catch((error) => notify(`读取替换规则失败：${errorText(error)}`, "error"));
  });
  if (disposed) { unlistenResources(); return; }
  unlisteners.push(unlistenResources);

  const unlistenRestore = await listen<AppBootstrap>("app-state-updated", (event) => {
    restoreRevision.value += 1;
    shelfReadRevision += 1;
    replacementRulesFeature.resetForRestore();
    void loadBootstrap(event.payload);
  });
  if (disposed) { unlistenRestore(); return; }
  unlisteners.push(unlistenRestore);
}

function openUrlImportFromRoute(): void {
  const value = route.query.importUrl;
  const url = typeof value === "string" ? value.trim() : "";
  if (!url || !/^https?:\/\//i.test(url)) return;
  if (replacementRulesFeature.openReplacementRulesUrlImport()) {
    replacementImportUrl.value = url;
    const query = { ...route.query };
    delete query.importUrl;
    void router.replace({ path: route.path, query });
  }
}

watch(() => route.query.importUrl, openUrlImportFromRoute, { immediate: true });

onMounted(() => {
  void (async () => {
    try {
      await listenForResourceUpdates();
      await loadBootstrap();
      openUrlImportFromRoute();
    } catch (error) {
      loadError.value = errorText(error);
      loadBusy.value = false;
    }
  })();
});

onBeforeUnmount(() => {
  disposed = true;
  for (const unlisten of unlisteners) unlisten();
  if (noticeTimer) clearTimeout(noticeTimer);
});

function returnToAdvancedSettings(): void {
  void router.push({ name: settingsRouteNames.other });
}

const query = ref("");

const sampleText = ref("");
const enabledCount = computed(() => rules.value.filter((rule) => rule.enabled).length);
const samplePreview = computed(() => {
  if (!draft.value?.pattern || !sampleText.value) return { output: "", error: "" };
  try {
    const source = draft.value;
    const output = source.isRegex
      ? sampleText.value.replace(new RegExp(source.pattern, "g"), source.replacement)
      : sampleText.value.split(source.pattern).join(source.replacement);
    return { output, error: "" };
  } catch (error) {
    return { output: "", error: error instanceof Error ? error.message : String(error) };
  }
});
const statusFilter = ref<RuleStatusFilter>("all");
const batchMode = ref(false);
const selectedIds = ref<string[]>([]);
const editingRule = ref<DisplayReplacementRule | null>(null);
const draft = ref<DisplayReplacementRule | null>(null);
const editorInitial = ref("");
const upstreamRuleChanged = ref(false);
const editorDirty = computed(() => Boolean(draft.value) && JSON.stringify(draft.value) !== editorInitial.value);
const discardPromptOpen = ref(false);
const pendingRuleSwitch = ref<DisplayReplacementRule | null>(null);
const pendingNewRule = ref(false);
let pendingRouteNavigation: ((allow: boolean) => void) | null = null;
onBeforeRouteLeave(() => {
  if (!draft.value) return true;
  if (busy.value || editorSaving.value) return false;
  if (!editorDirty.value) return true;
  return new Promise<boolean>((resolve) => {
    pendingRouteNavigation?.(false);
    pendingRouteNavigation = resolve;
    discardPromptOpen.value = true;
  });
});
onBeforeUnmount(() => {
  pendingRouteNavigation?.(false);
});
function keepRuleEditor(): void {
  discardPromptOpen.value = false;
  pendingRuleSwitch.value = null;
  pendingNewRule.value = false;
  const resume = pendingRouteNavigation;
  pendingRouteNavigation = null;
  resume?.(false);
}
function abandonRuleEditor(): void {
  if (busy.value || editorSaving.value) return;
  const nextRule = pendingRuleSwitch.value;
  const newRule = pendingNewRule.value;
  discardPromptOpen.value = false;
  pendingRuleSwitch.value = null;
  pendingNewRule.value = false;
  closeEditor();
  const resume = pendingRouteNavigation;
  pendingRouteNavigation = null;
  if (resume) resume(true);
  else if (nextRule) openEditor(nextRule);
  else if (newRule) createRule();
}
function requestCloseEditor(): void {
  if (busy.value || editorSaving.value) return;
  if (editorDirty.value) discardPromptOpen.value = true;
  else closeEditor();
}
const actionsMenuOpen = ref(false);
const addMenuOpen = ref(false);
const filterPanelOpen = ref(false);
const filterMenuPosition = ref({ top: 0, left: 0 });
const editorSaving = ref(false);
const editorSaveError = ref("");
const openRuleMenuId = ref<string | null>(null);
// The rules arrive asynchronously. Open the first editable rule once when
// it becomes available, without reopening an editor the user closed.
let desktopMounted = false;
let initialDesktopSelectionPending = true;
function openInitialDesktopRule(): void {
  if (!desktopMounted || !initialDesktopSelectionPending || busy.value || editorSaving.value
    || !window.matchMedia("(min-width: 861px)").matches || draft.value) return;
  const first = rules.value[0];
  if (!first) return;
  initialDesktopSelectionPending = false;
  openEditor(first);
}
onMounted(() => {
  desktopMounted = true;
  openInitialDesktopRule();
});
useOutsidePointerDismiss(
  () => addMenuOpen.value || actionsMenuOpen.value || filterPanelOpen.value || !!openRuleMenuId.value,
  (target) => target instanceof Element
    && !!target.closest(".replacement-add-wrap, .managed-list-toolbar__control, .managed-list-row__menu-wrap, .replacement-filter-panel"),
  () => {
  addMenuOpen.value = false;
  actionsMenuOpen.value = false;
  filterPanelOpen.value = false;
  openRuleMenuId.value = null;
  },
);

const filteredRules = computed(() => {
  const needle = query.value.trim().toLocaleLowerCase("zh-CN");
  return rules.value.filter((rule) => {
    if (statusFilter.value === "enabled" && !rule.enabled) return false;
    if (statusFilter.value === "disabled" && rule.enabled) return false;
    return !needle || `${rule.name} ${rule.pattern} ${rule.replacement} ${scopeLabel(rule.scope)}`
      .toLocaleLowerCase("zh-CN").includes(needle);
  });
});
const selectedRules = computed(() => rules.value.filter((rule) => selectedIds.value.includes(rule.id)));
const allVisibleSelected = computed(() => filteredRules.value.length > 0
  && filteredRules.value.every((rule) => selectedIds.value.includes(rule.id)));

watch(() => rules.value.map((rule) => rule.id), (ids) => {
  const existingIds = new Set(ids);
  const next = selectedIds.value.filter((id) => existingIds.has(id));
  if (next.length !== selectedIds.value.length) selectedIds.value = next;
});

const availableScopes = computed(() => {
  const values = new Map(scopeOptions.value.map((option) => [option.value, option.label]));
  if (!values.has("all")) values.set("all", "全部书籍");
  for (const rule of rules.value) {
    if (!values.has(rule.scope)) values.set(rule.scope, rule.scope);
  }
  if (draft.value && !values.has(draft.value.scope)) values.set(draft.value.scope, draft.value.scope);
  return [...values].map(([value, label]) => ({ value, label }));
});

watch(() => rules.value.map((rule) => rule.id), () => openInitialDesktopRule());
watch(busy, () => openInitialDesktopRule());

// A background refresh must not silently replace a partially edited rule.
watch(() => rules.value.find((rule) => rule.id === editingRule.value?.id), (current) => {
  if (!editingRule.value || !draft.value || editorSaving.value || upstreamRuleChanged.value) return;
  if (!current) {
    if (!editorDirty.value) closeEditor();
    else upstreamRuleChanged.value = true;
    return;
  }
  if (JSON.stringify(current) === JSON.stringify(editingRule.value)) return;
  if (editorDirty.value) {
    upstreamRuleChanged.value = true;
    return;
  }
  // Nothing typed by the user: refresh the clean editor to the latest rule.
  editingRule.value = { ...current };
  draft.value = { ...current };
  editorInitial.value = JSON.stringify(draft.value);
}, { deep: true });

function scopeLabel(scope: string): string {
  if (scope === "all") return "全部书籍";
  return scopeOptions.value.find((option) => option.value === scope)?.label ?? scope;
}

function openEditor(rule: DisplayReplacementRule): void {
  if (busy.value || editorSaving.value || (draft.value && editingRule.value?.id === rule.id)) return;
  if (editorDirty.value) {
    pendingRuleSwitch.value = rule;
    discardPromptOpen.value = true;
    return;
  }
  initialDesktopSelectionPending = false;
  editingRule.value = { ...rule };
  draft.value = { ...rule };
  editorInitial.value = JSON.stringify(draft.value);
  upstreamRuleChanged.value = false;
  editorSaveError.value = "";
}

function closeEditor(): void {
  upstreamRuleChanged.value = false;
  editingRule.value = null;
  draft.value = null;
  editorInitial.value = "";
  editorSaveError.value = "";
}

function createRule(): void {
  if (busy.value || editorSaving.value) return;
  if (editorDirty.value) {
    pendingNewRule.value = true;
    discardPromptOpen.value = true;
    return;
  }
  initialDesktopSelectionPending = false;
  addMenuOpen.value = false;
  actionsMenuOpen.value = false;
  editingRule.value = null;
  draft.value = {
    id: "",
    name: "",
    pattern: "",
    replacement: "",
    enabled: true,
    isRegex: false,
    scope: "all",
  };
  editorInitial.value = JSON.stringify(draft.value);
  upstreamRuleChanged.value = false;
  editorSaveError.value = "";
}

function reloadEditorRule(): void {
  if (!editingRule.value && draft.value) {
    draft.value = {
      ...draft.value,
      name: "",
      pattern: "",
      replacement: "",
      enabled: true,
      isRegex: false,
      scope: "all",
    };
    editorInitial.value = JSON.stringify(draft.value);
    editorSaveError.value = "";
    return;
  }
  const current = rules.value.find((rule) => rule.id === editingRule.value?.id);
  if (!current) {
    closeEditor();
    return;
  }
  editingRule.value = { ...current };
  draft.value = { ...current };
  editorInitial.value = JSON.stringify(draft.value);
  upstreamRuleChanged.value = false;
  editorSaveError.value = "";
}
async function saveEditor(): Promise<void> {
  if (!draft.value || busy.value || editorSaving.value || upstreamRuleChanged.value
    || (draft.value.enabled && !draft.value.pattern.trim())) return;
  const current = { ...draft.value, name: draft.value.name.trim() };
  if (!current.name) {
    editorSaveError.value = "请填写规则名称。";
    return;
  }
  editorSaving.value = true;
  editorSaveError.value = "";
  try {
    const updating = editingRule.value !== null;
    let savedId = current.id;
    let saved: boolean;
    if (updating) {
      saved = await replacementRulesFeature.saveReplacementRule(current.id, {
        name: current.name,
        pattern: current.pattern,
        replacement: current.replacement,
        enabled: current.enabled,
        isRegex: current.isRegex,
        scope: current.scope,
      });
    } else {
      const createdId = await replacementRulesFeature.createReplacementRule(current);
      saved = createdId !== null;
      if (createdId !== null) savedId = createdId;
    }
    if (saved) {
      const savedRule = { ...current, id: savedId };
      if (window.matchMedia("(min-width: 861px)").matches) {
        editingRule.value = savedRule;
        draft.value = { ...savedRule };
        editorInitial.value = JSON.stringify(draft.value);
      } else closeEditor();
    }
    else editorSaveError.value = "保存未完成，规则仍可修改或重试。";
  } catch (error) {
    editorSaveError.value = error instanceof Error ? error.message : String(error);
  } finally {
    editorSaving.value = false;
  }
}

function updateEnabled(rule: DisplayReplacementRule, enabled: boolean): void {
  if (busy.value || (enabled && !rule.pattern.trim())) return;
  replacementRulesFeature.updateReplacementRule(rule.id, { enabled });
}

function chooseAddAction(kind: "new" | "file" | "url"): void {
  addMenuOpen.value = false;
  if (busy.value || editorSaving.value) return;
  if (kind === "new") {
    createRule();
  }
  else if (kind === "file") void replacementRulesFeature.importReplacementRulesFromFile();
  else replacementRulesFeature.openReplacementRulesUrlImport();
}

function closeRowMenu(): void {
  openRuleMenuId.value = null;
}

function toggleSelection(rule: DisplayReplacementRule): void {
  if (busy.value || editorSaving.value) return;
  const selected = new Set(selectedIds.value);
  if (selected.has(rule.id)) selected.delete(rule.id);
  else selected.add(rule.id);
  selectedIds.value = [...selected];
}

function selectVisible(invert = false): void {
  if (busy.value || editorSaving.value) return;
  const selected = new Set(selectedIds.value);
  for (const rule of filteredRules.value) {
    if (invert) {
      if (selected.has(rule.id)) selected.delete(rule.id);
      else selected.add(rule.id);
    } else if (allVisibleSelected.value) {
      selected.delete(rule.id);
    } else {
      selected.add(rule.id);
    }
  }
  selectedIds.value = [...selected];
}

function requestBulkDelete(): void {
  if (busy.value || editorSaving.value || !selectedRules.value.length) return;
  void replacementRulesFeature.removeReplacements(selectedRules.value.map((rule) => rule.id));
}

function exportSelectedRules(): void {
  const rules = selectedRules.value;
  if (busy.value || editorSaving.value || !rules.length) return;
  const url = URL.createObjectURL(new Blob(
    [JSON.stringify({ schemaVersion: 1, rules }, null, 2)],
    { type: "application/json" },
  ));
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = "display-replacement-rules.json";
  document.body.append(anchor);
  try {
    anchor.click();
  } finally {
    anchor.remove();
    window.setTimeout(() => URL.revokeObjectURL(url), 1000);
  }
}

function toggleFilterPanel(event: MouseEvent): void {
  if (filterPanelOpen.value) {
    filterPanelOpen.value = false;
    return;
  }
  addMenuOpen.value = false;
  actionsMenuOpen.value = false;
  openRuleMenuId.value = null;
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
  filterPanelOpen.value = true;
}

</script>

<template>
  <section class="replacement-rules-page" :class="{ 'replacement-rules-page--empty': rules.length === 0 && !draft }">
    <header class="replacement-global-header">
      <BackButton label="返回高级设置" @click="returnToAdvancedSettings" />
      <div class="replacement-global-title"><strong>替换与净化</strong></div>
      <div class="replacement-add-wrap">
        <button type="button" class="replacement-primary-button replacement-global-add"
          :disabled="busy || editorSaving"
@click="addMenuOpen = !addMenuOpen; actionsMenuOpen = false; filterPanelOpen = false">
          <PrototypeIcon name="plus"/> 添加
        </button>
        <div v-if="addMenuOpen" class="replacement-add-options">
          <button type="button" @click="chooseAddAction('new')">新建配置</button>
          <button type="button" @click="chooseAddAction('file')">本地 JSON 导入</button>
          <button type="button" @click="chooseAddAction('url')">网络链接导入</button>
        </div>
      </div>
    </header>
    <div v-if="loadError" class="replacement-page-message error" role="alert">读取替换规则失败：{{ loadError }}</div>
    <div v-else-if="notice" class="replacement-page-message" :class="noticeKind" role="status">{{ notice }}</div>
    <div class="replacement-master">
      <div class="replacement-config-summary">
        <div><strong>{{ enabledCount }} 个启用</strong><small>{{ rules.length }} 个配置</small></div>

      </div>
      <ManagedListToolbar
        v-model:query="query"
        search-placeholder="替换净化-搜索"
        :filter-active="statusFilter !== 'all'"
        :filter-expanded="filterPanelOpen"
        :actions-expanded="actionsMenuOpen"
        @toggle-filter="toggleFilterPanel"
          @toggle-actions="actionsMenuOpen = !actionsMenuOpen; addMenuOpen = false; filterPanelOpen = false; openRuleMenuId = null"
      >
        <template #actions-menu>
          <div v-if="actionsMenuOpen" class="replacement-menu">
            <button type="button" :disabled="busy" @click="actionsMenuOpen = false; void replacementRulesFeature.reloadReplacementRules()">刷新规则</button>
            <button type="button" :disabled="busy || editorSaving" @click="batchMode = !batchMode; selectedIds = []; actionsMenuOpen = false">{{ batchMode ? '退出批量管理' : '批量管理' }}</button>
          </div>
        </template>
        <template #filter-menu>
          <Teleport to="body">
            <div v-if="filterPanelOpen" class="replacement-filter-panel" :style="{ top: filterMenuPosition.top + 'px', left: filterMenuPosition.left + 'px' }">
          <button type="button" @click="statusFilter = 'all'; filterPanelOpen = false">全部规则</button>
          <button type="button" @click="statusFilter = 'enabled'; filterPanelOpen = false">仅启用</button>
          <button type="button" @click="statusFilter = 'disabled'; filterPanelOpen = false">仅停用</button>
            </div>
          </Teleport>
        </template>
      </ManagedListToolbar>

      <div class="replacement-rule-list" @scroll.passive="filterPanelOpen = false; actionsMenuOpen = false; openRuleMenuId = null">
        <ManagedListRow
          v-for="rule in filteredRules"
          :key="rule.id"
          :name="rule.name || '未命名规则'"
          :subtitle="`${scopeLabel(rule.scope)} · ${rule.isRegex ? '正则' : '文本'}`"
          :enabled="rule.enabled"
          :active="editingRule?.id === rule.id && !!draft"
          :busy="busy || editorSaving"
          :batch-mode="batchMode"
          :selected="selectedIds.includes(rule.id)"
          :menu-open="openRuleMenuId === rule.id"
          @open="openEditor(rule)"
          @select="toggleSelection(rule)"
          @toggle-menu="openRuleMenuId = openRuleMenuId === rule.id ? null : rule.id; addMenuOpen = false; actionsMenuOpen = false; filterPanelOpen = false"
        >
            <template #menu>
              <button type="button" :disabled="busy || !rule.pattern.trim()" @click="closeRowMenu(); updateEnabled(rule, !rule.enabled)">{{ rule.enabled ? '停用' : '启用' }}</button>
              <button type="button" :disabled="busy" @click="closeRowMenu(); void replacementRulesFeature.removeReplacement(rule.id)">删除规则</button>
            </template>
        </ManagedListRow>
        <p v-if="loadBusy" class="replacement-rule-empty">正在读取规则…</p>
        <p v-else-if="!loadError && rules.length === 0" class="replacement-rule-empty">还没有规则</p>
        <p v-else-if="!loadError && !filteredRules.length" class="replacement-rule-empty">没有符合条件的规则</p>
      </div>
      <footer v-if="batchMode" class="replacement-selection-bar">
        <button type="button" class="replacement-text-button" :disabled="busy || editorSaving || !filteredRules.length" @click="selectVisible()">{{ allVisibleSelected ? '取消全选' : '全选' }}</button>
        <button type="button" class="replacement-text-button" :disabled="busy || editorSaving || !filteredRules.length" @click="selectVisible(true)">反选</button>
        <span class="replacement-selection-count">已选 {{ selectedRules.length }}</span>
        <button type="button" class="replacement-text-button" :disabled="busy || editorSaving || !selectedRules.length" @click="exportSelectedRules">导出所选</button>
        <button type="button" class="replacement-text-button replacement-delete-button" :disabled="busy || editorSaving || !selectedRules.length" @click="requestBulkDelete">删除</button>
      </footer>
    </div>
    <main v-if="draft" class="replacement-detail">
      <header class="replacement-editor-header">
        <BackButton label="返回规则列表" @click="requestCloseEditor" />
        <div class="replacement-page-title">
          <h1>{{ draft.name || (editingRule ? '未命名规则' : '新净化规则') }}</h1>
        </div>
        <label class="replacement-enabled-toggle"><span>启用</span><input v-model="draft.enabled" type="checkbox" :disabled="busy || !draft.pattern.trim()"/></label>
        <div class="replacement-editor-actions">
          <button type="button" class="replacement-primary-button replacement-mobile-save"
            :disabled="busy || editorSaving || upstreamRuleChanged || !draft.name.trim() || (draft.enabled && !draft.pattern.trim())"
            @click="saveEditor">{{ editorSaving ? '保存中…' : '保存' }}</button>
        </div>
      </header>
      <form class="replacement-editor-form" @submit.prevent="saveEditor">
        <section class="replacement-editor-section">
          <header><strong>规则</strong></header>
          <div class="replacement-editor-grid">
            <label class="replacement-field"><span>名称</span><input v-model="draft.name" maxlength="160" autocomplete="off" :disabled="busy" placeholder="规则名称"/></label>
            <label class="replacement-field"><span>作用范围</span><select v-model="draft.scope" :disabled="busy"><option v-for="scope in availableScopes" :key="scope.value" :value="scope.value">{{ scope.label }}</option></select></label>
          </div>
          <label class="replacement-field"><span>查找 / 正则</span><textarea v-model="draft.pattern" rows="4" spellcheck="false" :disabled="busy" placeholder="输入要查找的文字或正则表达式"/></label>
          <label class="replacement-field"><span>替换为</span><textarea v-model="draft.replacement" rows="4" spellcheck="false" :disabled="busy" placeholder="留空表示删除匹配内容"/></label>
          <div class="replacement-editor-options"><label><input v-model="draft.isRegex" type="checkbox" :disabled="busy"/> 使用正则表达式</label></div>
          <p v-if="draft.enabled && !draft.pattern.trim()" class="replacement-validation">填写匹配内容后才能启用规则。</p>
        </section>
        <section class="replacement-editor-section">
          <header><strong>测试</strong></header>
          <label class="replacement-field"><span>输入文本</span><textarea v-model="sampleText" rows="3" spellcheck="false" placeholder="输入一段正文用于测试"/></label>
          <div class="replacement-test-result">
            <template v-if="samplePreview.error">预览失败：{{ samplePreview.error }}</template>
            <template v-else-if="sampleText && draft.pattern">{{ samplePreview.output || '（空文本：匹配内容已全部删除）' }}</template>
            <template v-else>请输入文本查看替换效果</template>
          </div>
          <p class="replacement-preview-note">即时预览由浏览器计算，实际规则执行以 Rust 引擎为准。</p>
        </section>
        <div v-if="upstreamRuleChanged" class="replacement-editor-upstream">
          <span>这条规则已从外部更新或删除，当前未保存草稿仍保留，但不能直接覆盖新内容。重新读取将放弃当前草稿。</span>
          <button type="button" class="replacement-secondary-button" :disabled="busy || editorSaving" @click="reloadEditorRule">重新读取规则</button>
        </div>
        <p v-if="editorSaveError" class="replacement-validation">{{ editorSaveError }}</p>
        <div class="replacement-editor-bottom">
          <span></span>
          <button type="button" class="replacement-secondary-button"
            :disabled="busy || editorSaving || !editorDirty" @click="reloadEditorRule">还原</button>
          <button type="submit" class="replacement-primary-button" :disabled="busy || editorSaving || upstreamRuleChanged || !draft.name.trim() || (draft.enabled && !draft.pattern.trim())">{{ editorSaving ? '保存中…' : editingRule ? '保存修改' : '添加规则' }}</button>
        </div>
      </form>
    </main>
    <div v-if="replacementImportOpen" class="replacement-import-scrim"
      @click.self="!replacementImportBusy && replacementRulesFeature.cancelReplacementRulesUrlImport()">
      <section class="replacement-import-dialog">
        <h3 id="replacement-import-title">从网络链接导入</h3>
        <form class="replacement-import-url-form" @submit.prevent="void replacementRulesFeature.importReplacementRulesByUrl()">
          <label for="replacement-import-url">替换与净化规则 JSON 链接</label>
          <input id="replacement-import-url" v-model="replacementImportUrl" type="url" required
            inputmode="url" autocomplete="url" maxlength="4096"
            placeholder="https://example.com/replacement-rules.json"
            :disabled="replacementImportBusy"
            @input="replacementImportError = ''" />
          <button type="submit" class="replacement-primary-button" :disabled="busy || replacementImportBusy || !replacementImportUrl.trim()">
            {{ replacementImportBusy ? '下载并导入中…' : '下载并导入' }}
          </button>
          <small>下载完成后会直接导入规则。</small>
        </form>
        <p v-if="replacementImportError" class="replacement-import-error">{{ replacementImportError }}</p>
        <div class="replacement-import-actions">
          <button type="button" class="replacement-secondary-button" :disabled="replacementImportBusy" @click="replacementRulesFeature.cancelReplacementRulesUrlImport()">取消</button>
        </div>
      </section>
    </div>
    <div v-if="discardPromptOpen" class="replacement-discard-mask"
      @click.self="keepRuleEditor">
      <section class="replacement-discard-dialog">
        <h3 id="replacement-discard-title">放弃未保存的净化规则？</h3>
        <p>当前规则的内容已经修改，{{ pendingNewRule ? '创建新规则前必须先放弃当前草稿。' : '返回或切换配置会丢弃这些草稿。' }}</p>
        <div>
          <button type="button" class="replacement-secondary-button" @click="keepRuleEditor">继续编辑</button>
          <button type="button" class="replacement-primary-button replacement-discard-danger" @click="abandonRuleEditor">放弃修改</button>
        </div>
      </section>
    </div>
  </section>
</template>

<style scoped>
.replacement-rules-page {
  --rr-bg: var(--app-surface);
  --rr-ink: var(--app-text);
  --rr-muted: var(--app-muted);
  --rr-line: var(--app-line);
  --rr-soft: var(--app-panel-softer);
  --rr-accent: var(--app-accent);
  --red: var(--app-danger);
  min-width: 0;
  min-height: 0;
  height: 100%;
  display: flex;
  flex-direction: column;
  gap: 0;
  padding: 0;
  overflow: hidden;
  background: var(--app-surface);
  color: var(--rr-ink);
}
.replacement-editor-header { min-width:0; display:flex; align-items:center; gap:10px; }
.replacement-page-title { min-width:0; display:flex; flex:1; flex-direction:column; gap:3px; }
.replacement-page-title h1 { margin:0; color:var(--rr-ink); font-size:17px; line-height:1.25; font-weight:700; }
.replacement-field input,.replacement-field select,.replacement-field textarea { min-width:0; min-height:38px; padding:0 10px; border:1px solid var(--rr-line); border-radius:8px; color:var(--rr-ink); background:var(--rr-bg); font:inherit; font-size:13px; }
.replacement-menu { position:absolute; z-index:5; top:calc(100% + 5px); right:0; min-width:170px; display:grid; gap:2px; padding:5px; border:1px solid var(--rr-line); border-radius:10px; background:var(--rr-bg); box-shadow:0 10px 28px rgb(30 42 35 / 15%); }
.replacement-filter-panel { position:fixed; z-index:1000; box-sizing:border-box; display:grid; width:min(170px,calc(100vw - 16px)); max-height:calc(100vh - 16px); overflow-y:auto; padding:5px; border:1px solid var(--rr-line,var(--app-line)); border-radius:11px; background:#fff; box-shadow:0 12px 28px #22223322; }
.replacement-filter-panel button { display:block; width:100%; min-height:34px; padding:0 10px; border:0; border-radius:7px; color:var(--rr-ink,#343640); background:transparent; text-align:left; font:inherit; font-size:12px; cursor:pointer; }
.replacement-filter-panel button:hover { background:#f1eefd; }
.replacement-menu button { min-height:34px; padding:0 9px; border:0; border-radius:6px; color:var(--rr-ink); background:transparent; text-align:left; font:inherit; font-size:12px; cursor:pointer; }
.replacement-menu button:hover:not(:disabled) { background:var(--rr-soft); }
.replacement-menu button:disabled { opacity:.45; cursor:default; }
.replacement-rule-list { min-width:0;min-height:0;flex:1 1 auto;display:grid;gap:3px;margin-right:-14px;padding:0 14px 0 0;overflow-y:auto;overscroll-behavior:contain; }
.replacement-page-message { padding:7px 14px; color:var(--rr-muted); font-size:12px; }
.replacement-page-message.error { color:var(--red); }
.replacement-text-button,.replacement-secondary-button,.replacement-primary-button { min-height:34px; padding:0 11px; border:1px solid var(--rr-line); border-radius:8px; color:var(--rr-ink); background:var(--rr-bg); font:inherit; font-size:12px; cursor:pointer; }
.replacement-text-button { border-color:transparent; background:transparent; }
.replacement-primary-button { border-color:var(--rr-accent); color:#fff; background:var(--rr-accent); font-weight:650; }
.replacement-text-button:hover:not(:disabled),.replacement-secondary-button:hover:not(:disabled) { background:var(--rr-soft); }
.replacement-text-button:disabled,.replacement-secondary-button:disabled,.replacement-primary-button:disabled { opacity:.45; cursor:default; }
.replacement-editor-actions { display:flex; align-items:center; gap:5px; margin-left:auto; }
.replacement-field { min-width:0; display:grid; gap:6px; color:var(--rr-muted); font-size:12px; font-weight:600; }
.replacement-field input,.replacement-field select { width:100%; }
.replacement-field textarea { width:100%; min-height:100px; padding:9px 10px; resize:vertical; font-family:ui-monospace,SFMono-Regular,Consolas,monospace; line-height:1.55; }
.replacement-editor-options { display:flex; flex-wrap:wrap; gap:18px; color:var(--rr-ink); font-size:13px; }
.replacement-editor-options label { display:flex; align-items:center; gap:7px; }
.replacement-editor-options input { accent-color:var(--rr-accent); }
.replacement-validation { margin:0; color:var(--red); font-size:12px; }

.replacement-discard-mask{position:fixed;inset:0;z-index:120;display:grid;place-items:center;padding:16px;background:#1b1a2a88}
.replacement-import-scrim{position:fixed;inset:0;z-index:2000;display:grid;place-items:center;padding:16px;background:#191b2780}
.replacement-import-dialog{box-sizing:border-box;width:min(420px,100%);max-height:min(82dvh,700px);overflow:auto;padding:22px;border:1px solid var(--rr-line);border-radius:14px;background:#fff;box-shadow:0 20px 40px #11112233}
.replacement-import-dialog h3{margin:0;font-size:17px}
.replacement-import-url-form{display:grid;gap:9px;margin:10px 0 14px}
.replacement-import-url-form label{font-size:12px;font-weight:650}
.replacement-import-url-form input{box-sizing:border-box;width:100%;min-height:39px;padding:9px 11px;border:1px solid var(--rr-line);border-radius:9px;background:#fff;color:var(--rr-ink);font:inherit;font-size:12px}
.replacement-import-url-form button{justify-self:start}
.replacement-import-url-form small{color:var(--rr-muted);font-size:11px;line-height:1.5}
.replacement-import-error{margin:9px 0 18px;color:var(--red);font-size:12px;line-height:1.5}
.replacement-import-actions{display:flex;justify-content:flex-end;gap:8px}
.replacement-discard-dialog{box-sizing:border-box;width:min(430px,100%);padding:23px;border-radius:15px;border:1px solid #e5e6ed;background:#fff;box-shadow:0 20px 42px #11112232}
.replacement-discard-dialog h3{margin:0 0 10px;font-size:16px;color:#333542}
.replacement-discard-dialog p{margin:0;color:#7e8290;font-size:13px;line-height:1.65}
.replacement-discard-dialog>div{display:flex;justify-content:flex-end;gap:9px;margin-top:20px}
.replacement-discard-danger{background:#bc5964}
.replacement-editor-form input:disabled,.replacement-editor-form textarea:disabled,.replacement-editor-form select:disabled{cursor:not-allowed}

/* Content engine configuration workspace, matched to prototype editorPage('rules'). */
.replacement-global-header{
  position:sticky;top:0;z-index:35;grid-column:1/-1;display:grid;
  grid-template-columns:40px minmax(0,1fr) auto;align-items:center;gap:10px;
  box-sizing:border-box;min-height:64px;padding:0 max(14px,calc((100% - 1180px)/2));
  border-bottom:1px solid var(--rr-line);background:#f6f7f9f0;backdrop-filter:blur(14px);
}
.replacement-global-title{min-width:0}
.replacement-global-title strong{font-size:14px;color:#242631}
.replacement-add-wrap{position:relative;min-width:0}
.replacement-global-add {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  border-color: var(--rr-accent);
  background: var(--rr-accent);
  color: #fff;
  font-size: 11px;
}
.replacement-global-add .prototype-icon{width:16px;height:16px}
.replacement-add-options{position:absolute;right:0;top:calc(100% + 7px);z-index:55;display:grid;gap:3px;width:232px;padding:7px;border:1px solid var(--rr-line);border-radius:11px;background:#fff;box-shadow:0 16px 40px #20203322}
.replacement-add-options>button{display:flex;align-items:center;min-height:38px;padding:9px;border:0;border-radius:8px;background:transparent;color:#343640;text-align:left;font-size:11px;font-weight:650;cursor:pointer}
.replacement-add-options > button:hover {
  background: var(--app-accent-soft);
}
.replacement-config-summary{display:flex;justify-content:space-between;gap:8px;padding:8px 5px 10px;margin:0}
.replacement-config-summary>div{display:flex;justify-content:space-between;gap:10px;width:100%}
.replacement-config-summary strong{color:#979ba5;font-size:9px;font-weight:500}
.replacement-config-summary small{color:#979ba5;font-size:9px}
.replacement-selection-bar{position:static;z-index:4;display:flex;flex:none;align-items:center;gap:4px;flex-wrap:wrap;margin-top:auto;padding:8px 4px;border-top:1px solid var(--rr-line);background:#fafafd}
.replacement-selection-count{margin-left:auto;color:var(--rr-muted);font-size:9px;white-space:nowrap}
.replacement-delete-button{color:#b7545e}
.replacement-rule-empty{align-self:center;justify-self:center;margin:auto;color:var(--rr-muted);font-size:12px;text-align:center}
.replacement-detail{min-width:0;min-height:0;overflow-y:auto;padding:28px 34px 48px;background:#fff}
.replacement-editor-header{position:static;margin:0 0 20px;background:transparent;gap:10px}
.replacement-editor-header>.back-button{display:none}
.replacement-editor-header .replacement-page-title{flex:1;min-width:0;gap:3px}
.replacement-editor-header .replacement-page-title h1{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:17px}
.replacement-enabled-toggle{display:flex;align-items:center;gap:8px;color:#6d707c;font-size:9px;white-space:nowrap}
.replacement-enabled-toggle input {
  width: 17px;
  height: 17px;
  accent-color: var(--rr-accent);
}
.replacement-editor-actions{margin-left:0}
.replacement-mobile-save{display:none}
.replacement-editor-form{display:block;padding:0;border:0;border-radius:0;background:transparent}
.replacement-editor-section{padding:18px 0;border-top:1px solid var(--rr-line)}
.replacement-editor-section>header{display:grid;gap:3px;margin-bottom:12px}
.replacement-editor-section>header strong{font-size:11px}
.replacement-editor-grid{display:grid;grid-template-columns:1fr 1fr;gap:12px}
.replacement-editor-section .replacement-field{margin-bottom:14px;font-size:9px;font-weight:500;color:#777c85}
.replacement-editor-section .replacement-field input,
.replacement-editor-section .replacement-field select{box-sizing:border-box;height:36px;font-size:9px}
.replacement-editor-section .replacement-field textarea{min-height:84px;padding:9px;font-size:9px}
.replacement-editor-options{margin:0 0 2px;font-size:9px}
.replacement-test-result{min-height:42px;padding:12px;border:1px solid var(--rr-line);border-radius:10px;background:#f1f1f5;font-size:9px;line-height:1.6;white-space:pre-wrap;overflow-wrap:anywhere}
.replacement-preview-note{margin:8px 0 0;color:var(--rr-muted);font-size:9px}
.replacement-editor-bottom{display:grid;grid-template-columns:1fr auto auto;gap:8px;align-items:center;padding-top:16px;border-top:1px solid var(--rr-line)}
.replacement-editor-bottom .replacement-secondary-button{background:#eceef2;border-color:transparent}
.replacement-editor-upstream{display:flex;align-items:center;gap:12px;justify-content:space-between;flex-wrap:wrap;padding:11px 12px;background:#fff6ed;border:1px solid #eed6b5;border-radius:10px;color:#855f31;font-size:12px;line-height:1.55}
.replacement-editor-upstream>span{flex:1;min-width:200px}
.replacement-master{display:flex;flex-direction:column;min-width:0;min-height:0;height:100%;max-height:none;overflow:hidden}
@media(min-width:861px){
  .replacement-rules-page{display:grid;grid-template-columns:300px minmax(0,1fr);grid-template-rows:64px minmax(0,1fr);align-items:start}
  .replacement-global-header{grid-row:1;grid-column:1/-1}
  .replacement-master{grid-row:2;grid-column:1;min-width:0;box-sizing:border-box;height:100%;max-height:none;overflow:hidden;padding:14px;border-right:1px solid var(--rr-line);background:#fafafd}
  .replacement-detail{grid-row:2;grid-column:2}
  .replacement-rules-page--empty .replacement-master{grid-column:1/-1;border-right:0}
}
@media(max-width:860px){
  .replacement-rules-page{display:flex;flex-direction:column}
  .replacement-global-header{box-sizing:border-box;min-height:64px;padding:0 14px}
  .replacement-global-title strong{font-size:13px}
  .replacement-master{height:auto;flex:1 1 auto;padding:12px 14px;min-width:0}
  .replacement-rules-page:has(.replacement-detail)>.replacement-global-header,
  .replacement-rules-page:has(.replacement-detail)>.replacement-master{display:none}
  .replacement-detail{height:auto;flex:1 1 auto;padding:0 14px 66px}
  .replacement-editor-header{position:sticky;top:0;z-index:32;box-sizing:border-box;min-height:64px;margin:0 -14px 15px;padding:0 14px;border-bottom:1px solid var(--rr-line);background:#f6f7f9f5;backdrop-filter:blur(12px)}
  .replacement-editor-header>.back-button{display:inline-flex}
  .replacement-editor-header .replacement-page-title{display:flex;flex:1;min-width:0;flex-direction:column;gap:2px}
  .replacement-editor-header .replacement-page-title h1{font-size:12px;line-height:1.25}
  .replacement-enabled-toggle{margin-left:auto}
  .replacement-mobile-save{display:inline-flex;align-items:center;min-height:33px;margin-left:3px}
  .replacement-editor-grid{grid-template-columns:1fr}
  .replacement-editor-bottom{grid-template-columns:1fr auto auto;gap:6px}
}
@media(max-width:480px){
  .replacement-add-options{width:min(240px,calc(100vw - 30px))}
  .replacement-editor-section .replacement-field input,
  .replacement-editor-section .replacement-field textarea,
  .replacement-editor-section .replacement-field select{font-size:11px}
}

</style>

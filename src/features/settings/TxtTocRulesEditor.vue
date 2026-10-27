<script setup lang="ts">
import { Dialog as KDialog, DialogButton as KDialogButton, Popover as KPopover, Toggle as KToggle, Button as KButton, Glass as KGlass } from "konsta/vue";
import SubpageHeader from "../../ui/SubpageHeader.vue";
// 列表使用全局规则；表单只编辑当前规则的草稿。
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { onBeforeRouteLeave } from "vue-router";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { settingsRouteNames } from "../../router";
import { returnToRoute } from "../../app/routeReturn";
import ManagedListRow from "../../ui/ManagedListRow.vue";
import ManagedListToolbar from "../../ui/ManagedListToolbar.vue";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import { useOutsidePointerDismiss } from "../../ui/useOutsidePointerDismiss";
import { txtTocRules as rules, txtTocRuleStore } from "./configurationStores";
import {
  prepareConfigurationImport,
  type TxtTocRule,
  type TxtTocRulesDocument,
} from "../../api/settings";
import type { AppBootstrap } from "../../api/types";
import { notify } from "../../app/notifications";

interface RuleDraft {
  name: string;
  rule: string;
  example: string;
  enable: boolean;
}

function returnToAdvancedSettings(): void {
  returnToRoute(settingsRouteNames.other);
}


const errorMessage = ref("");
const editingId = ref<string | null>(null);
const editingSerialNumber = ref(0);
const initialSerialNumber = ref(0);
const validPriority = computed(() => Number.isInteger(editingSerialNumber.value)
  && editingSerialNumber.value >= -2147483648 && editingSerialNumber.value <= 2147483647);
const showEditor = ref(false);
const query = ref("");
const batchMode = ref(false);
const importUrl = ref("");
const importError = ref("");
const importBusy = ref(false);
const importDialogOpen = ref(false);
let restoreRevision = 0;
let disposed = false;
const unlisteners: UnlistenFn[] = [];
const testText = ref("");
const enabledRuleCount = computed(() => rules.value.filter((item) => item.enable).length);
const testMatches = computed(() => {
  const lines = testText.value.split(/\r?\n/).map((line) => line.trim()).filter(Boolean);
  if (!lines.length || !draft.value.rule.trim()) return { count: 0, lines: [] as string[], error: "" };
  try {
    // Approximate Rust's full-line heading matcher. Rust still validates the regex and titles on save/import.
    const pattern = new RegExp(draft.value.rule, "mu");
    const matching = lines.flatMap((line) => {
      const match = pattern.exec(line);
      if (!match || line.slice(0, match.index).trim() || line.slice(match.index + match[0].length).trim()) return [];
      const title = (match[1] ?? match[0]).trim();
      return title && [...title].length <= 200 ? [title] : [];
    });
    return { count: matching.length, lines: matching, error: "" };
  } catch (error) {
    return { count: 0, lines: [] as string[], error: error instanceof Error ? error.message : String(error) };
  }
});
const enabledFilter = ref<"all" | "enabled" | "disabled">("all");
const selectedIds = ref<string[]>([]);
const deleteTargets = ref<TxtTocRule[]>([]);
const openRuleMenuId = ref<string | null>(null);
const actionsMenuOpen = ref(false);
const addMenuOpen = ref(false);
const initialRuleSnapshot = ref("");
const filterPanelOpen = ref(false);
const filterMenuPosition = ref({ top: 0, left: 0 });
const draft = ref<RuleDraft>(emptyDraft());
const initialDraft = ref(JSON.stringify(emptyDraft()));
const editorDirty = computed(() => showEditor.value
  && (JSON.stringify(draft.value) !== initialDraft.value || editingSerialNumber.value !== initialSerialNumber.value));
const upstreamRuleChanged = computed(() => {
  const id = editingId.value;
  if (!id || !initialRuleSnapshot.value) return false;
  const current = rules.value.find(item => item.id === id);
  return !current || JSON.stringify(current) !== initialRuleSnapshot.value;
});
const discardPromptOpen = ref(false);
const pendingSwitchRule = ref<TxtTocRule | null>(null);
const pendingNewRule = ref(false);

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

let resolveNavigation: ((allow: boolean) => void) | null = null;
onBeforeRouteLeave(() => {
  if (!showEditor.value) return true;
  if (busy.value) return false;
  if (!editorDirty.value) return true;
  return new Promise<boolean>((resolve) => {
    resolveNavigation?.(false);
    resolveNavigation = resolve;
    discardPromptOpen.value = true;
  });
});
onBeforeUnmount(() => {
  disposed = true;
  for (const unlisten of unlisteners) unlisten();
  resolveNavigation?.(false);
});
useOutsidePointerDismiss(
  () => addMenuOpen.value || actionsMenuOpen.value || filterPanelOpen.value || !!openRuleMenuId.value,
  (target) => target instanceof Element
    && !!target.closest(".txt-toc-editor__add-options, .txt-toc-editor__top-actions, .managed-list-toolbar__control, .managed-list-row__menu-wrap, .txt-toc-editor__filter-menu"),
  () => {
  addMenuOpen.value = false;
  actionsMenuOpen.value = false;
  filterPanelOpen.value = false;
  openRuleMenuId.value = null;
  },
);
function cancelDiscard(): void {
  discardPromptOpen.value = false;
  pendingSwitchRule.value = null;
  pendingNewRule.value = false;
  const resolve = resolveNavigation;
  resolveNavigation = null;
  resolve?.(false);
}
function discardDraft(): void {
  if (busy.value) return;
  discardPromptOpen.value = false;
  const switchingRule = pendingSwitchRule.value;
  const creatingNew = pendingNewRule.value;
  pendingSwitchRule.value = null;
  pendingNewRule.value = false;
  resetEditor();
  const resolve = resolveNavigation;
  resolveNavigation = null;
  if (resolve) resolve(true);
  else if (switchingRule) editRule(switchingRule);
  else if (creatingNew) createRule();
}
function requestCancelEditor(): void {
  if (busy.value) return;
  if (editorDirty.value) discardPromptOpen.value = true;
  else resetEditor();
}

const orderedRules = computed(() => [...rules.value].sort((left, right) =>
  left.serialNumber - right.serialNumber || left.id.localeCompare(right.id),
));
const visibleRules = computed(() => {
  const needle = query.value.trim().toLocaleLowerCase("zh-CN");
  return orderedRules.value.filter((rule) => {
    if (enabledFilter.value === "enabled" && !rule.enable) return false;
    if (enabledFilter.value === "disabled" && rule.enable) return false;
    return !needle || `${rule.name} ${rule.rule} ${rule.example ?? ""}`.toLocaleLowerCase("zh-CN").includes(needle);
  });
});
const selectedRules = computed(() => orderedRules.value.filter((rule) => selectedIds.value.includes(rule.id)));
const allVisibleSelected = computed(() => visibleRules.value.length > 0
  && visibleRules.value.every((rule) => selectedIds.value.includes(rule.id)));
const busy = importBusy;
const atRuleLimit = computed(() => editingId.value === null && rules.value.length >= 64);
const canSave = computed(() =>
  !busy.value && !upstreamRuleChanged.value && !atRuleLimit.value && validPriority.value
  && draft.value.name.trim().length > 0 && draft.value.rule.trim().length > 0,
);

onMounted(async () => {
  const unlisten = await listen<AppBootstrap>("app-state-updated", () => { restoreRevision++; resetEditor(); });
  if (disposed) unlisten(); else unlisteners.push(unlisten);
  if (window.matchMedia("(min-width: 861px)").matches && orderedRules.value.length) editRule(orderedRules.value[0]);
});
watch(() => rules.value.map((rule) => rule.id), (ids) => {
  const existingIds = new Set(ids);
  const next = selectedIds.value.filter((id) => existingIds.has(id));
  if (next.length !== selectedIds.value.length) selectedIds.value = next;
});

function emptyDraft(): RuleDraft {
  return { name: "", rule: "", example: "", enable: true };
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function resetEditor(): void {
  showEditor.value = false;
  testText.value = "";
  editingId.value = null;
  initialRuleSnapshot.value = "";
  editingSerialNumber.value = orderedRules.value.length;
  initialSerialNumber.value = editingSerialNumber.value;
  draft.value = emptyDraft();
  initialDraft.value = JSON.stringify(draft.value);
  errorMessage.value = "";
  openRuleMenuId.value = null;
}

function editRule(rule: TxtTocRule): void {
  if (busy.value || (showEditor.value && editingId.value === rule.id)) return;
  if (editorDirty.value) {
    pendingSwitchRule.value = rule;
    discardPromptOpen.value = true;
    return;
  }
  openRuleMenuId.value = null;
  actionsMenuOpen.value = false;
  addMenuOpen.value = false;
  editingId.value = rule.id;
  initialRuleSnapshot.value = JSON.stringify(rule);
  showEditor.value = true;
  editingSerialNumber.value = rule.serialNumber;
  initialSerialNumber.value = rule.serialNumber;
  draft.value = {
    name: rule.name,
    rule: rule.rule,
    example: rule.example ?? "",
    enable: rule.enable,
  };
  initialDraft.value = JSON.stringify(draft.value);
  testText.value = rule.example ?? "";
  errorMessage.value = "";
}

function restoreEditorDraft(): void {
  if (busy.value || !showEditor.value) return;
  const original = editingId.value ? rules.value.find(rule => rule.id === editingId.value) : null;
  if (editingId.value && !original) {
    errorMessage.value = "当前规则已被删除，请返回列表并重新选择。";
    return;
  }
  resetEditor();
  if (original) editRule(original);
  else createRule();
}
function selectAddAction(action: "new" | "file" | "url"): void {
  addMenuOpen.value = false;
  if (action === "new") createRule();
  else if (action === "file") openLocalImport();
  else openUrlImport();
}
function createRule(): void {
  if (busy.value || atRuleLimit.value) return;
  if (editorDirty.value) {
    pendingNewRule.value = true;
    discardPromptOpen.value = true;
    return;
  }
  resetEditor();
  showEditor.value = true;
  actionsMenuOpen.value = false;
  addMenuOpen.value = false;
  draft.value = emptyDraft();
}

function toggleSelection(rule: TxtTocRule): void {
  if (busy.value) return;
  const selected = new Set(selectedIds.value);
  if (selected.has(rule.id)) selected.delete(rule.id);
  else selected.add(rule.id);
  selectedIds.value = [...selected];
}

function canMoveRule(rule: TxtTocRule, direction: -1 | 1): boolean {
  if (query.value.trim() || enabledFilter.value !== "all") return false;
  const index = orderedRules.value.findIndex((item) => item.id === rule.id);
  const targetIndex = index + direction;
  return index >= 0 && targetIndex >= 0 && targetIndex < orderedRules.value.length;
}

function selectVisible(invert = false): void {
  if (busy.value) return;
  const selected = new Set(selectedIds.value);
  for (const rule of visibleRules.value) {
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

function exportSelectedRules(): void {
  const selected = selectedRules.value;
  if (busy.value || !selected.length) return;
  const document: TxtTocRulesDocument = { schemaVersion: 1, rules: selected };
  const url = URL.createObjectURL(new Blob(
    [JSON.stringify(document, null, 2)],
    { type: "application/json;charset=utf-8" },
  ));
  const anchor = window.document.createElement("a");
  anchor.href = url;
  anchor.download = "txt-toc-rules-selected.json";
  window.document.body.append(anchor);
  try {
    anchor.click();
    notify(`已导出 ${selected.length} 条目录规则。`);
    errorMessage.value = "";
  } finally {
    anchor.remove();
    window.setTimeout(() => URL.revokeObjectURL(url), 1000);
  }
}

async function openLocalImport(): Promise<void> {
  if (busy.value || importBusy.value) return;
  actionsMenuOpen.value = false;
  importError.value = "";
  errorMessage.value = "";
  const revision = restoreRevision;
  importBusy.value = true;
  try {
    const result = await prepareConfigurationImport<TxtTocRulesDocument>("txtTocRules", { kind: "local" });
    if (!result || disposed || revision !== restoreRevision) return;
    appendImportedRules(result);
    notify(`已从本地文件导入 ${result.rules.length} 条 TXT 规则。`);
  } catch (error) {
    errorMessage.value = `本地导入失败：${errorText(error)}`;
  } finally {
    importBusy.value = false;
  }
}

function openUrlImport(): void {
  if (busy.value || importBusy.value) return;
  actionsMenuOpen.value = false;
  importUrl.value = "";
  importError.value = "";
  importDialogOpen.value = true;
}

watch(importUrl, () => {
  importError.value = "";
});

async function importUrlRules(): Promise<void> {
  const url = importUrl.value.trim();
  if (!url || importBusy.value || busy.value) return;
  importError.value = "";
  errorMessage.value = "";
  const revision = restoreRevision;
  importBusy.value = true;
  try {
    const result = await prepareConfigurationImport<TxtTocRulesDocument>("txtTocRules", { kind: "url", url });
    if (!result || disposed || revision !== restoreRevision) return;
    appendImportedRules(result);
    importDialogOpen.value = false;
    importUrl.value = "";
    notify(`已从网络链接导入 ${result.rules.length} 条 TXT 规则。`);
  } catch (error) {
    importError.value = `网络导入失败：${errorText(error)}`;
  } finally {
    importBusy.value = false;
  }
}

function closeUrlImport(): void {
  if (importBusy.value) return;
  importDialogOpen.value = false;
  importUrl.value = "";
  importError.value = "";
}

function saveRule(): void {
  if (busy.value || !showEditor.value) return;
  if (upstreamRuleChanged.value) {
    errorMessage.value = "规则已在其他操作中更新，不能用旧草稿覆盖。请还原并重新编辑。";
    return;
  }
  errorMessage.value = "";
  if (!draft.value.name.trim()) {
    errorMessage.value = "请填写规则名称。";
    return;
  }
  if (!draft.value.rule.trim()) {
    errorMessage.value = "正则表达式不能为空。";
    return;
  }
  if (!validPriority.value) {
    errorMessage.value = "优先级必须是 32 位整数，数字越小越优先。";
    return;
  }

  const item: TxtTocRule = {
    id: editingId.value ?? `txt-toc-${crypto.randomUUID()}`,
    name: draft.value.name.trim(), rule: draft.value.rule,
    ...(draft.value.example.trim() ? { example: draft.value.example } : {}),
    serialNumber: editingSerialNumber.value, enable: draft.value.enable,
  };
  txtTocRuleStore.update(item);
  resetEditor();
  if (window.matchMedia("(min-width: 861px)").matches) editRule(item);
}

function toggleRule(rule: TxtTocRule, enable: boolean): void {
  if (busy.value) return;
  txtTocRuleStore.update({ ...rule, enable });
}
function moveRule(ruleId: string, direction: -1 | 1): void {
  if (busy.value) return;
  const reordered = orderedRules.value;
  const index = reordered.findIndex((rule) => rule.id === ruleId);
  const targetIndex = index + direction;
  if (index < 0 || targetIndex < 0 || targetIndex >= reordered.length) return;
  [reordered[index], reordered[targetIndex]] = [reordered[targetIndex], reordered[index]];
  errorMessage.value = "";
  reordered.forEach((rule, index) => {
    if (rule.serialNumber !== index) txtTocRuleStore.update({ ...rule, serialNumber: index });
  });
}

function requestDelete(rule: TxtTocRule): void {
  if (busy.value) return;
  openRuleMenuId.value = null;
  deleteTargets.value = [rule];
  errorMessage.value = "";
}

function requestBulkDelete(): void {
  if (busy.value || !selectedRules.value.length) return;
  deleteTargets.value = [...selectedRules.value];
  errorMessage.value = "";
}

function cancelDelete(): void {
  if (busy.value) return;
  deleteTargets.value = [];
  errorMessage.value = "";
}

function confirmDelete(): void {
  const targets = [...deleteTargets.value];
  if (!targets.length || busy.value) return;
  errorMessage.value = "";
  try {
    targets.forEach(rule => txtTocRuleStore.remove(rule.id));
    deleteTargets.value = [];
    const targetIds = new Set(targets.map((rule) => rule.id));
    selectedIds.value = selectedIds.value.filter((id) => !targetIds.has(id));
    if (targets.some((rule) => editingId.value === rule.id)) resetEditor();
    notify(targets.length === 1
      ? `“${targets[0].name}”已删除。`
      : `已删除 ${targets.length} 条规则。`);
  } catch (error) {
    errorMessage.value = errorText(error);
    const existingIds = new Set(rules.value.map((rule) => rule.id));
    selectedIds.value = selectedIds.value.filter((id) => existingIds.has(id));
    deleteTargets.value = deleteTargets.value.filter((rule) => existingIds.has(rule.id));
  }
}

function appendImportedRules(document: TxtTocRulesDocument): void {
  if (rules.value.length + document.rules.length > 64) throw new Error("TXT 目录规则最多保存 64 条。");
  const first = Math.max(0, ...rules.value.map(rule => rule.serialNumber)) + 1;
  if (first + document.rules.length - 1 > 2147483647) throw new Error("规则优先级超出 32 位整数范围。");
  document.rules.forEach((rule, index) => txtTocRuleStore.update({ ...rule, serialNumber: first + index }));
}
</script>

<template>
  <section class="txt-toc-editor my-settings-subpage">
    <SubpageHeader class="txt-toc-editor__topbar col-span-full row-start-1" title="TXT 目录识别" back-label="返回高级设置" @back="returnToAdvancedSettings" >
<template #actions><div class="txt-toc-editor__top-actions">
        <KButton rounded inline type="button" class="txt-toc-editor__button txt-toc-editor__button--primary gap-2"
          :disabled="busy || atRuleLimit"
          @click="addMenuOpen = !addMenuOpen; actionsMenuOpen = false; filterPanelOpen = false; openRuleMenuId = null">
          <PrototypeIcon name="plus"/> 添加
        </KButton>

      </div></template>
</SubpageHeader>
<KPopover v-if="addMenuOpen" opened :backdrop="false" target=".txt-toc-editor__top-actions" class="txt-toc-editor__add-options w-60 max-w-[calc(100vw-32px)]">
  <div class="p-2">
<KButton rounded inline clear class="gap-2 w-full justify-start text-current" type="button" @click="selectAddAction('new')">新建配置</KButton>
<KButton rounded inline clear class="gap-2 w-full justify-start text-current" type="button" @click="selectAddAction('file')">本地 JSON 导入</KButton>
<KButton rounded inline clear class="gap-2 w-full justify-start text-current" type="button" @click="selectAddAction('url')">网络链接导入</KButton>
  </div>
</KPopover>
    <div class="txt-toc-editor__body" :class="{ 'txt-toc-editor__body--empty': rules.length === 0 }">
      <aside class="txt-toc-editor__master">
        <header class="txt-toc-editor__summary">
          <div><strong>{{ enabledRuleCount }} 个启用</strong><small>{{ rules.length }} / 64 个配置</small></div>

        </header>
        <ManagedListToolbar
          v-model:query="query"
          search-placeholder="搜索目录规则"
          :filter-expanded="filterPanelOpen"
          :actions-expanded="actionsMenuOpen"
          @toggle-filter="toggleFilterPanel"
          @toggle-actions="actionsMenuOpen = !actionsMenuOpen; filterPanelOpen = false; addMenuOpen = false; openRuleMenuId = null"
        >
          <template #actions-menu>
            <div v-if="actionsMenuOpen" class="txt-toc-editor__menu txt-toc-editor__menu--right">
              <KButton rounded inline clear class="gap-2 w-full justify-start text-current" type="button" @click="batchMode = !batchMode; selectedIds = []; actionsMenuOpen = false">{{ batchMode ? '退出批量管理' : '批量管理' }}</KButton>
              <span class="txt-toc-editor__menu-hint">正则表达式由 Rust 在保存时校验</span>
            </div>
          </template>
          <template #filter-menu>
            <Teleport to="body">
              <KGlass component="div" :highlight="false" v-if="filterPanelOpen" class="txt-toc-editor__filter-menu rounded-2xl" :style="{ top: filterMenuPosition.top + 'px', left: filterMenuPosition.left + 'px' }">
                <KButton rounded inline clear class="gap-2 w-full justify-start text-current" type="button" @click="enabledFilter = 'all'; filterPanelOpen = false">全部规则</KButton>
                <KButton rounded inline clear class="gap-2 w-full justify-start text-current" type="button" @click="enabledFilter = 'enabled'; filterPanelOpen = false">仅启用</KButton>
                <KButton rounded inline clear class="gap-2 w-full justify-start text-current" type="button" @click="enabledFilter = 'disabled'; filterPanelOpen = false">仅停用</KButton>
              </KGlass>
            </Teleport>
          </template>
        </ManagedListToolbar>
        <div class="txt-toc-editor__list-scroll" @scroll.passive="filterPanelOpen = false; actionsMenuOpen = false; openRuleMenuId = null">
        <p v-if="atRuleLimit" class="txt-toc-editor__notice">最多保存 64 条规则；删除一条后可以继续添加。</p>
        <p v-if="rules.length === 0" class="txt-toc-editor__no-rules">还没有规则</p>
        <div v-else-if="!visibleRules.length" class="txt-toc-editor__empty">
          <strong>没有符合条件的规则</strong>
          <p>调整搜索条件以查看规则。</p>
        </div>
        <div v-else class="txt-toc-editor__list">
          <ManagedListRow
            v-for="rule in visibleRules"
            :key="rule.id"
            :name="rule.name || '未命名规则'"
            :subtitle="`优先级 ${rule.serialNumber} · ${rule.enable ? '已启用' : '已停用'}`"
            :enabled="rule.enable"
            :busy="busy"
            :batch-mode="batchMode"
            :selected="selectedIds.includes(rule.id)"
            :menu-open="openRuleMenuId === rule.id"
            @open="editRule(rule)"
            @select="toggleSelection(rule)"
            @toggle-menu="openRuleMenuId = openRuleMenuId === rule.id ? null : rule.id; addMenuOpen = false; actionsMenuOpen = false; filterPanelOpen = false"
          >
              <template #menu>
                <KButton rounded inline clear class="gap-2" type="button" :disabled="busy" @click="openRuleMenuId = null; toggleRule(rule, !rule.enable)">{{ rule.enable ? '停用' : '启用' }}</KButton>
                <KButton rounded inline clear class="gap-2" type="button" :disabled="busy || !canMoveRule(rule, -1)" @click="openRuleMenuId = null; moveRule(rule.id, -1)">上移</KButton>
                <KButton rounded inline clear class="gap-2" type="button" :disabled="busy || !canMoveRule(rule, 1)" @click="openRuleMenuId = null; moveRule(rule.id, 1)">下移</KButton>
                <KButton rounded inline clear type="button" class="is-danger k-color-brand-danger gap-2 " :disabled="busy" @click="requestDelete(rule)">删除规则</KButton>
              </template>
          </ManagedListRow>
        </div>
        </div>
        <footer v-if="batchMode" class="txt-toc-editor__selection-bar">
          <KButton rounded inline clear type="button" class="txt-toc-editor__text-button gap-2" :disabled="busy || !visibleRules.length" @click="selectVisible()">{{ allVisibleSelected ? '取消全选' : '全选' }}</KButton>
          <KButton rounded inline clear type="button" class="txt-toc-editor__text-button gap-2" :disabled="busy || !visibleRules.length" @click="selectVisible(true)">反选</KButton>
          <span class="txt-toc-editor__selection-count">已选 {{ selectedRules.length }}</span>
          <KButton rounded inline clear type="button" class="txt-toc-editor__text-button gap-2" :disabled="busy || !selectedRules.length" @click="exportSelectedRules">导出</KButton>
          <KButton rounded inline clear type="button" class="txt-toc-editor__text-button txt-toc-editor__delete-button k-color-brand-danger gap-2 " :disabled="busy || !selectedRules.length" @click="requestBulkDelete">删除</KButton>
        </footer>
      </aside>
      <main v-if="showEditor" class="txt-toc-editor__detail" data-back-page>
        <SubpageHeader class="txt-toc-editor__editor-header col-span-full row-start-1" :title="editingId ? draft.name || '未命名目录规则' : '新目录规则'" back-label="返回规则列表" :back-disabled="busy" @back="requestCancelEditor" local-back>
<template #actions><label class="txt-toc-editor__switchline">
            <span>启用</span><KToggle component="span" :checked="!!draft.enable" @change="draft.enable = ($event.target as HTMLInputElement).checked" :disabled="busy" />
          </label>
<KButton rounded inline type="button" class="txt-toc-editor__button txt-toc-editor__button--primary txt-toc-editor__mobile-save gap-2"
            :disabled="!canSave" @click="saveRule">保存</KButton></template>
</SubpageHeader>
        <form class="txt-toc-editor__form" @submit.prevent="saveRule">
          <div v-if="upstreamRuleChanged" class="txt-toc-editor__stale">
            当前规则已被外部操作更新或删除。草稿仍在，但不能直接保存覆盖。请选择还原或取消编辑。
          </div>
          <section class="txt-toc-editor__editor-section">
            <header><strong>识别规则</strong></header>
            <div class="txt-toc-editor__field-grid">
              <label class="txt-toc-editor__field"><span>名称</span><input v-model="draft.name" type="text" maxlength="256" autocomplete="off" :disabled="busy" placeholder="规则名称" /></label>
              <label class="txt-toc-editor__field txt-toc-editor__priority"><span>优先级</span>
                  <input v-model.number="editingSerialNumber" type="number" step="1" min="-2147483648" max="2147483647" :disabled="busy"/>
                </label>
            </div>
            <label class="txt-toc-editor__field"><span>正则表达式</span><textarea v-model="draft.rule" rows="4" spellcheck="false" autocomplete="off" :disabled="busy" placeholder="^第.+章.*$"></textarea></label>
            <label class="txt-toc-editor__field"><span>示例章节标题（可选）</span><input v-model="draft.example" type="text" maxlength="2048" autocomplete="off" :disabled="busy" placeholder="例如：第1章 山水之间" /></label>
          </section>
          <section class="txt-toc-editor__editor-section">
            <header><strong>测试文本</strong></header>
            <label class="txt-toc-editor__field"><span>待识别标题</span><textarea v-model="testText" rows="4" spellcheck="false" :disabled="busy" placeholder="第1章 山水之间&#10;第2章 长夜"></textarea></label>
            <div class="txt-toc-editor__test-result">
              <template v-if="testMatches.error">预览无法解析此表达式：{{ testMatches.error }}</template>
              <template v-else>预览识别到 <strong>{{ testMatches.count }}</strong> 行<span v-if="testMatches.lines.length">：{{ testMatches.lines.join('、') }}</span></template>
            </div>
          </section>
          <p v-if="!validPriority" class="txt-toc-editor__limit">优先级必须填写 32 位整数。</p>
          <p v-if="atRuleLimit" class="txt-toc-editor__limit">已达到 64 条规则上限。</p>
          <div v-if="errorMessage" class="txt-toc-editor__error">{{ errorMessage }}</div>
          <div class="txt-toc-editor__form-actions">
            <KButton rounded inline clear v-if="editingId" type="button" class="txt-toc-editor__button txt-toc-editor__button--danger k-color-brand-danger gap-2 " :disabled="busy" @click="requestDelete(orderedRules.find((item) => item.id === editingId)!)">删除</KButton>
            <span class="txt-toc-editor__action-spacer"></span>
            <KButton rounded inline tonal type="button" class="txt-toc-editor__button txt-toc-editor__button--secondary gap-2"
              :disabled="busy || (!editorDirty && !upstreamRuleChanged)"
              @click="restoreEditorDraft">还原</KButton>
            <KButton rounded inline type="submit" class="txt-toc-editor__button txt-toc-editor__button--primary gap-2" :disabled="!canSave">{{ editingId ? '保存修改' : '添加规则' }}</KButton>
          </div>
        </form>
      </main>
    </div>
    <div v-if="importDialogOpen" class="txt-toc-editor__scrim" data-back-layer
      @click.self="closeUrlImport">
      <section class="txt-toc-editor__confirm txt-toc-editor__import-dialog">
        <h3 id="txt-toc-import-title">从网络链接导入</h3>
        <form class="txt-toc-editor__import-url-form" @submit.prevent="importUrlRules">
          <label for="txt-toc-import-url">TXT 目录规则 JSON 链接</label>
          <input id="txt-toc-import-url" v-model="importUrl" type="url" required
            :disabled="importBusy" placeholder="https://example.com/txt-toc.json" autocomplete="url" />
          <KButton rounded inline type="submit" class="txt-toc-editor__button txt-toc-editor__button--primary gap-2" :disabled="importBusy || !importUrl.trim()">
            {{ importBusy ? '下载并导入中…' : '下载并导入' }}
          </KButton>
          <small>下载完成后会直接导入规则，上限 1 MiB。</small>
        </form>
        <p v-if="importError" class="txt-toc-editor__error">{{ importError }}</p>
        <div class="txt-toc-editor__confirm-actions">
          <KButton rounded inline tonal type="button" class="txt-toc-editor__button txt-toc-editor__button--secondary gap-2" :disabled="importBusy" data-back-action @click="closeUrlImport">取消</KButton>
        </div>
      </section>
    </div>
    <div v-if="deleteTargets.length" class="txt-toc-editor__scrim" data-back-layer
      @click.self="cancelDelete">
      <KDialog opened :backdrop="false" class="txt-toc-editor__confirm w-[min(420px,calc(100vw-32px))] max-h-[88dvh] overflow-auto">
  <template #title><h3 id="txt-toc-delete-title">{{ deleteTargets.length === 1 ? '删除这条规则？' : '删除选中的 ' + deleteTargets.length + ' 条规则？' }}</h3></template>
<p id="txt-toc-delete-description">{{ deleteTargets.length === 1 ? '“' + deleteTargets[0].name + '”将从本地 TXT 目录识别设置中移除。' : '选中的规则将从本地 TXT 目录识别设置中移除。' }}</p>
<div v-if="errorMessage" class="txt-toc-editor__error">{{ errorMessage }}</div><template #buttons><KDialogButton type="button" :disabled="busy" data-back-action @click="cancelDelete">取消</KDialogButton>
<KDialogButton type="button" :disabled="busy" @click="confirmDelete" class="k-color-brand-danger" strong>保存</KDialogButton></template>
</KDialog>
    </div>
    <div v-if="discardPromptOpen" class="txt-toc-editor__scrim" data-back-layer
      @click.self="cancelDiscard">
      <KDialog opened :backdrop="false" class="txt-toc-editor__confirm w-[min(420px,calc(100vw-32px))] max-h-[88dvh] overflow-auto">
  <template #title><h3 id="txt-toc-discard-title">放弃未保存的规则？</h3></template>
<p>当前名称、表达式、优先级或启用状态已改变，离开编辑器会丢弃这些修改。</p><template #buttons><KDialogButton type="button" data-back-action @click="cancelDiscard">继续编辑</KDialogButton>
<KDialogButton type="button" @click="discardDraft" class="k-color-brand-danger" strong>放弃修改</KDialogButton></template>
</KDialog>
    </div>
  </section>
</template>
<style scoped>
@layer components {
.txt-toc-editor{--txt-accent:var(--app-accent);--txt-text:var(--app-text);--txt-muted:var(--app-subtle);--txt-line:var(--app-line);--txt-soft:var(--app-panel-softer);display:grid;grid-template-rows:auto minmax(0,1fr);width:100%;height:100%;min-width:0;min-height:0;overflow:hidden;color:var(--txt-text)}
.txt-toc-editor__top-actions{position:relative;display:flex;justify-content:flex-end}
.txt-toc-editor__top-actions>.txt-toc-editor__button{gap:5px;min-height:34px;font-size:var(--ui-font-body)}
.txt-toc-editor__top-actions>.txt-toc-editor__button :deep(svg){width:15px;height:15px}
.txt-toc-editor__stale{margin:0 0 15px;padding:12px;border:1px solid var(--app-warning-line);border-radius:10px;background:var(--app-panel-soft);color:var(--app-warning);font-size:var(--ui-font-caption);line-height:1.6}
.txt-toc-editor__body{display:grid;grid-template-columns:1fr;min-width:0;height:100%;min-height:0;overflow:hidden;background:var(--app-background)}
.txt-toc-editor__body--empty{grid-template-columns:minmax(0,1fr)}
.txt-toc-editor__master{display:flex;flex-direction:column;min-width:0;height:100%;min-height:0;overflow:hidden;padding:14px;background:var(--app-background)}
.txt-toc-editor__summary{display:flex;align-items:center;justify-content:space-between;gap:12px;margin-bottom:10px;padding:3px}
.txt-toc-editor__summary>div{display:flex;align-items:center;justify-content:space-between;gap:12px;width:100%}
.txt-toc-editor__summary strong{font-size:var(--ui-font-caption);font-weight:500;color:var(--txt-muted)}
.txt-toc-editor__summary small{color:var(--txt-muted);font-size:var(--ui-font-caption)}
.txt-toc-editor__add{display:inline-flex;align-items:center;gap:4px}
.txt-toc-editor__add .prototype-icon{width:16px;height:16px}
.txt-toc-editor__menu{position:absolute;z-index:20;top:calc(100% + 5px);left:0;min-width:170px;padding:5px;border:1px solid var(--txt-line);border-radius:11px;background:var(--app-surface);box-shadow:0 12px 28px #22223322}
.txt-toc-editor__menu--right{left:auto;right:0}
.txt-toc-editor__menu button{width:100%;min-height:34px;padding:0 10px;border:0;border-radius:7px;background:transparent;text-align:left;font:inherit;font-size:var(--ui-font-body);cursor:pointer}
.txt-toc-editor__menu button:disabled{opacity:.4;cursor:default}
.txt-toc-editor__menu button.is-danger{color:var(--app-danger)}
.txt-toc-editor__filter-menu{position:fixed;z-index:1000;box-sizing:border-box;display:grid;width:min(170px,calc(100vw - 16px));max-height:calc(100vh - 16px);overflow-y:auto;padding:5px;border:1px solid var(--txt-line,var(--app-line));border-radius:11px;background:var(--app-surface);box-shadow:0 12px 28px #22223322}
.txt-toc-editor__filter-menu button{width:100%;min-height:34px;padding:0 10px;border:0;border-radius:7px;background:transparent;text-align:left;font:inherit;font-size:var(--ui-font-body);cursor:pointer}
.txt-toc-editor__menu-hint{display:block;padding:8px 10px;color:var(--txt-muted);font-size:var(--ui-font-caption);line-height:1.5;border-top:1px solid var(--txt-line)}
.txt-toc-editor__list{display:grid;gap:3px}
.txt-toc-editor__list-scroll{display:flex;flex:1 1 auto;flex-direction:column;min-height:0;margin-right:-14px;padding-right:14px;overflow-y:auto;overscroll-behavior:contain}
.txt-toc-editor__no-rules{display:grid;flex:1 1 auto;place-items:center;margin:0;color:var(--txt-muted);font-size:var(--ui-font-caption)}
.txt-toc-editor__empty{padding:24px 10px;text-align:center;color:var(--txt-muted);font-size:var(--ui-font-caption)}
.txt-toc-editor__empty strong{display:block;color:var(--txt-text);font-size:var(--ui-font-caption)}
.txt-toc-editor__empty p{line-height:1.55}
.txt-toc-editor__detail{min-width:0;height:100%;min-height:0;overflow-y:auto;padding:28px 34px 50px;background:var(--app-background)}
.txt-toc-editor__mobile-save{display:none}
.txt-toc-editor__switchline{display:inline-flex;align-items:center;gap:7px;white-space:nowrap;color:var(--app-muted);font-size:var(--ui-font-caption)}
.txt-toc-editor__switchline input{width:18px;height:18px;accent-color:var(--txt-accent)}
.txt-toc-editor__form{margin-top:18px}
.txt-toc-editor__editor-section{padding:18px 0;border-top:1px solid var(--txt-line)}
.txt-toc-editor__editor-section>header{display:grid;gap:4px;margin-bottom:14px}
.txt-toc-editor__editor-section>header strong{font-size:var(--ui-font-caption)}
.txt-toc-editor__field-grid{display:grid;grid-template-columns:1fr 1fr;gap:12px}
.txt-toc-editor__field{min-width:0;display:grid;gap:6px;margin-bottom:14px;color:var(--app-muted);font-size:var(--ui-font-caption);font-weight:600}
.txt-toc-editor__field input,.txt-toc-editor__field textarea{width:100%;min-width:0;box-sizing:border-box;padding:8px 9px;border:1px solid var(--txt-line);border-radius:9px;background:var(--app-surface);color:var(--txt-text);font:inherit;font-size:var(--ui-font-body);font-weight:400;line-height:1.5}
.txt-toc-editor__field input{height:36px}
.txt-toc-editor__field textarea{resize:vertical;min-height:88px;font-family:ui-monospace,SFMono-Regular,Consolas,monospace}
.txt-toc-editor__field input:focus,.txt-toc-editor__field textarea:focus{outline:2px solid var(--app-accent);outline-offset:1px;border-color:var(--txt-accent)}
.txt-toc-editor__priority input{font-variant-numeric:tabular-nums}
.txt-toc-editor__test-result{padding:12px;border:1px solid var(--txt-line);border-radius:10px;background:var(--app-panel-soft);font-size:var(--ui-font-caption);line-height:1.7;overflow-wrap:anywhere}
.txt-toc-editor__test-result strong{color:var(--txt-accent)}
.txt-toc-editor__form-actions{display:flex;align-items:center;gap:8px;padding-top:18px;border-top:1px solid var(--txt-line)}
.txt-toc-editor__action-spacer{flex:1}
.txt-toc-editor__button{display:inline-flex;justify-content:center;align-items:center;min-height:34px;padding:0 12px;border:0;border-radius:9px;font:inherit;font-size:var(--ui-font-body);font-weight:700;cursor:pointer}
.txt-toc-editor__button:disabled{opacity:.45;cursor:default}
.txt-toc-editor__button--primary{background:var(--txt-accent);color:var(--app-on-accent)}
.txt-toc-editor__button--secondary{background:var(--app-panel-soft);color:var(--app-muted)}
.txt-toc-editor__button--danger{background:var(--app-danger-soft);color:var(--app-danger)}
.txt-toc-editor__message,.txt-toc-editor__error,.txt-toc-editor__notice,.txt-toc-editor__limit{margin:8px 0;padding:10px 12px;border-radius:9px;font-size:var(--ui-font-caption);line-height:1.5}
.txt-toc-editor__notice,.txt-toc-editor__message{background:var(--app-panel-soft);color:var(--app-muted)}
.txt-toc-editor__error{background:var(--app-danger-soft);color:var(--app-danger)}
.txt-toc-editor__limit{color:var(--app-danger)}
.txt-toc-editor__selection-bar{display:flex;flex:none;align-items:center;gap:5px;margin-top:auto;padding:8px 0;border-top:1px solid var(--txt-line);background:var(--app-background)}
.txt-toc-editor__text-button{padding:8px;border:0;border-radius:8px;background:transparent;font-size:var(--ui-font-body);cursor:pointer}
.txt-toc-editor__selection-count{margin-left:auto;color:var(--txt-muted);font-size:var(--ui-font-caption)}
.txt-toc-editor__delete-button{color:var(--app-danger)}
.txt-toc-editor__scrim{position:fixed;z-index:2000;inset:0;display:grid;place-items:center;padding:16px;background:var(--app-scrim)}
.txt-toc-editor__confirm-actions{display:flex;justify-content:flex-end;gap:8px}
@media(min-width:861px){.txt-toc-editor__body{grid-template-columns:300px minmax(0,1fr)}.txt-toc-editor__master{border-right:1px solid var(--txt-line)}}
@media(max-width:860px){
 .txt-toc-editor__master{padding:16px var(--ui-page-gutter)}
 .txt-toc-editor__detail{position:fixed;inset:0;z-index:40;height:auto;padding:0 var(--ui-page-gutter) calc(32px + var(--safe-bottom))}
 .txt-toc-editor__switchline input{width:16px;height:16px}
 .txt-toc-editor__mobile-save{display:inline-flex}
 .txt-toc-editor__field-grid{grid-template-columns:1fr}
 .txt-toc-editor__form-actions{flex-wrap:wrap}
}

.txt-toc-editor__import-dialog{max-height:min(82dvh,700px);overflow:auto}
.txt-toc-editor__import-url-form{display:grid;gap:9px;margin:10px 0 14px}
.txt-toc-editor__import-url-form label{font-size:var(--ui-font-body);font-weight:650}
.txt-toc-editor__import-url-form input{min-height:39px;width:100%;box-sizing:border-box;padding:9px 11px;border:1px solid var(--txt-line);border-radius:9px;background:var(--app-surface);color:var(--txt-text);font:inherit;font-size:var(--ui-font-body)}
.txt-toc-editor__import-url-form button{justify-self:start}
.txt-toc-editor__import-url-form small{font-size:var(--ui-font-caption);color:var(--txt-muted);line-height:1.5}
}
</style>

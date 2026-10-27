<script setup lang="ts">
import { Dialog as KDialog, DialogButton as KDialogButton, Popover as KPopover, Toggle as KToggle, Button as KButton, Glass as KGlass } from "konsta/vue";
import SubpageHeader from "../../ui/SubpageHeader.vue";
// All pages share the rule document; this form owns only its edit draft.
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { onBeforeRouteLeave, useRoute } from "vue-router";
import router, { settingsRouteNames } from "../../router";
import { returnToRoute } from "../../app/routeReturn";
import { shelfBooks as shelf } from "../shelf/shelfBooks";
import type { AppBootstrap, DisplayReplacementRule } from "../../api/types";
import ManagedListRow from "../../ui/ManagedListRow.vue";
import ManagedListToolbar from "../../ui/ManagedListToolbar.vue";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import { useOutsidePointerDismiss } from "../../ui/useOutsidePointerDismiss";
import { useReplacementRules } from "./useReplacementRules";
import { replacementRules as rules } from "./configurationStores";

defineOptions({ inheritAttrs: false });

type RuleStatusFilter = "all" | "enabled" | "disabled";

const route = useRoute();

const restoreRevision = ref(0);
const loadError = ref("");
const notice = ref("");
const noticeKind = ref<"success" | "error">("success");
let noticeTimer: ReturnType<typeof setTimeout> | undefined;
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
const replacementImportOpen = replacementRulesFeature.replacementImportOpen;
const replacementImportUrl = replacementRulesFeature.replacementImportUrl;
const replacementImportError = replacementRulesFeature.replacementImportError;
const replacementImportBusy = replacementRulesFeature.replacementImportBusy;
const busy = replacementImportBusy;
const scopeOptions = computed(() => [
  { value: "all", label: "全部书籍" },
  ...shelf.value.books.map((book) => ({ value: `book:${book.id}`, label: book.title })),
]);

async function listenForResourceUpdates(): Promise<void> {
  const unlistenRestore = await listen<AppBootstrap>("app-state-updated", () => {
    restoreRevision.value++;
    replacementRulesFeature.resetForRestore();
  });
  if (disposed) unlistenRestore(); else unlisteners.push(unlistenRestore);
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
      openUrlImportFromRoute();
    } catch (error) {
      loadError.value = errorText(error);
        }
  })();
});

onBeforeUnmount(() => {
  disposed = true;
  for (const unlisten of unlisteners) unlisten();
  if (noticeTimer) clearTimeout(noticeTimer);
});

function returnToAdvancedSettings(): void {
  returnToRoute(settingsRouteNames.other);
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
const editingRuleId = ref<string | null>(null);
const editingRule = computed(() => rules.value.find(rule => rule.id === editingRuleId.value) ?? null);
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
  if (busy.value) return false;
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
  if (busy.value) return;
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
  if (busy.value) return;
  if (editorDirty.value) discardPromptOpen.value = true;
  else closeEditor();
}
const actionsMenuOpen = ref(false);
const addMenuOpen = ref(false);
const filterPanelOpen = ref(false);
const filterMenuPosition = ref({ top: 0, left: 0 });
const editorSaveError = ref("");
const openRuleMenuId = ref<string | null>(null);
// The rules arrive asynchronously. Open the first editable rule once when
// it becomes available, without reopening an editor the user closed.
let desktopMounted = false;
let initialDesktopSelectionPending = true;
function openInitialDesktopRule(): void {
  if (!desktopMounted || !initialDesktopSelectionPending || busy.value
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
    && !!target.closest(".replacement-add-options, .replacement-add-wrap, .managed-list-toolbar__control, .managed-list-row__menu-wrap, .replacement-filter-panel"),
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
watch(() => rules.value.find((rule) => rule.id === editingRuleId.value), (current) => {
  if (!editingRuleId.value || !draft.value || upstreamRuleChanged.value) return;
  if (!current) {
    if (!editorDirty.value) closeEditor();
    else upstreamRuleChanged.value = true;
    return;
  }
  if (JSON.stringify(current) === editorInitial.value) return;
  if (editorDirty.value) {
    upstreamRuleChanged.value = true;
    return;
  }
  // Nothing typed by the user: refresh the clean editor to the latest rule.
  editingRuleId.value = current.id;
  draft.value = { ...current };
  editorInitial.value = JSON.stringify(draft.value);
}, { deep: true });

function scopeLabel(scope: string): string {
  if (scope === "all") return "全部书籍";
  return scopeOptions.value.find((option) => option.value === scope)?.label ?? scope;
}

function openEditor(rule: DisplayReplacementRule): void {
  if (busy.value || (draft.value && editingRule.value?.id === rule.id)) return;
  if (editorDirty.value) {
    pendingRuleSwitch.value = rule;
    discardPromptOpen.value = true;
    return;
  }
  initialDesktopSelectionPending = false;
  editingRuleId.value = rule.id;
  draft.value = { ...rule };
  editorInitial.value = JSON.stringify(draft.value);
  upstreamRuleChanged.value = false;
  editorSaveError.value = "";
}

function closeEditor(): void {
  upstreamRuleChanged.value = false;
  editingRuleId.value = null;
  draft.value = null;
  editorInitial.value = "";
  editorSaveError.value = "";
}

function createRule(): void {
  if (busy.value) return;
  if (editorDirty.value) {
    pendingNewRule.value = true;
    discardPromptOpen.value = true;
    return;
  }
  initialDesktopSelectionPending = false;
  addMenuOpen.value = false;
  actionsMenuOpen.value = false;
  editingRuleId.value = null;
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
  const current = rules.value.find((rule) => rule.id === editingRuleId.value);
  if (!current) {
    closeEditor();
    return;
  }
  editingRuleId.value = current.id;
  draft.value = { ...current };
  editorInitial.value = JSON.stringify(draft.value);
  upstreamRuleChanged.value = false;
  editorSaveError.value = "";
}
function saveEditor(): void {
  if (!draft.value || busy.value || upstreamRuleChanged.value
    || (draft.value.enabled && !draft.value.pattern.trim())) return;
  const current = { ...draft.value, name: draft.value.name.trim() };
  if (!current.name) {
    editorSaveError.value = "请填写规则名称。";
    return;
  }
  editorSaveError.value = "";
  const id = editingRule.value
    ? (replacementRulesFeature.saveReplacementRule(current.id, current), current.id)
    : replacementRulesFeature.createReplacementRule(current);
  if (window.matchMedia("(min-width: 861px)").matches) {
    editingRuleId.value = id;
    draft.value = { ...current, id };
    editorInitial.value = JSON.stringify(draft.value);
  } else closeEditor();
}

function updateEnabled(rule: DisplayReplacementRule, enabled: boolean): void {
  if (busy.value || (enabled && !rule.pattern.trim())) return;
  replacementRulesFeature.updateReplacementRule(rule.id, { enabled });
}

function chooseAddAction(kind: "new" | "file" | "url"): void {
  addMenuOpen.value = false;
  if (busy.value) return;
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
  if (busy.value) return;
  const selected = new Set(selectedIds.value);
  if (selected.has(rule.id)) selected.delete(rule.id);
  else selected.add(rule.id);
  selectedIds.value = [...selected];
}

function selectVisible(invert = false): void {
  if (busy.value) return;
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
  if (busy.value || !selectedRules.value.length) return;
  void replacementRulesFeature.removeReplacements(selectedRules.value.map((rule) => rule.id));
}

function exportSelectedRules(): void {
  const rules = selectedRules.value;
  if (busy.value || !rules.length) return;
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
    <SubpageHeader class="replacement-global-header col-span-full row-start-1" title="替换与净化" back-label="返回高级设置" @back="returnToAdvancedSettings" >
<template #actions><div class="replacement-add-wrap">
        <KButton rounded inline type="button" class="replacement-primary-button replacement-global-add gap-2"
          :disabled="busy"
@click="addMenuOpen = !addMenuOpen; actionsMenuOpen = false; filterPanelOpen = false">
          <PrototypeIcon name="plus"/> 添加
        </KButton>

      </div></template>
</SubpageHeader>
<KPopover v-if="addMenuOpen" opened :backdrop="false" target=".replacement-add-wrap" class="replacement-add-options w-60 max-w-[calc(100vw-32px)]">
  <div class="p-2">
<KButton rounded inline clear class="gap-2 w-full justify-start text-current" type="button" @click="chooseAddAction('new')">新建配置</KButton>
<KButton rounded inline clear class="gap-2 w-full justify-start text-current" type="button" @click="chooseAddAction('file')">本地 JSON 导入</KButton>
<KButton rounded inline clear class="gap-2 w-full justify-start text-current" type="button" @click="chooseAddAction('url')">网络链接导入</KButton>
  </div>
</KPopover>
    <div v-if="loadError" class="replacement-page-message error" role="alert">读取替换规则失败：{{ loadError }}</div>
    <div v-else-if="notice" class="replacement-page-message" :class="noticeKind" role="status">{{ notice }}</div>
    <div class="replacement-master">
      <div class="replacement-config-summary">
        <div><strong>{{ enabledCount }} 个启用</strong><small>{{ rules.length }} 个配置</small></div>

      </div>
      <ManagedListToolbar
        v-model:query="query"
        search-placeholder="替换净化-搜索"
        :filter-expanded="filterPanelOpen"
        :actions-expanded="actionsMenuOpen"
        @toggle-filter="toggleFilterPanel"
          @toggle-actions="actionsMenuOpen = !actionsMenuOpen; addMenuOpen = false; filterPanelOpen = false; openRuleMenuId = null"
      >
        <template #actions-menu>
          <KGlass component="div" :highlight="false" v-if="actionsMenuOpen" class="replacement-menu rounded-2xl">
            <KButton rounded inline clear class="gap-2 w-full justify-start text-current" type="button" :disabled="busy" @click="batchMode = !batchMode; selectedIds = []; actionsMenuOpen = false">{{ batchMode ? '退出批量管理' : '批量管理' }}</KButton>
          </KGlass>
        </template>
        <template #filter-menu>
          <Teleport to="body">
            <KGlass component="div" :highlight="false" v-if="filterPanelOpen" class="replacement-filter-panel rounded-2xl" :style="{ top: filterMenuPosition.top + 'px', left: filterMenuPosition.left + 'px' }">
          <KButton rounded inline clear class="gap-2 w-full justify-start text-current" type="button" @click="statusFilter = 'all'; filterPanelOpen = false">全部规则</KButton>
          <KButton rounded inline clear class="gap-2 w-full justify-start text-current" type="button" @click="statusFilter = 'enabled'; filterPanelOpen = false">仅启用</KButton>
          <KButton rounded inline clear class="gap-2 w-full justify-start text-current" type="button" @click="statusFilter = 'disabled'; filterPanelOpen = false">仅停用</KButton>
            </KGlass>
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
          :busy="busy"
          :batch-mode="batchMode"
          :selected="selectedIds.includes(rule.id)"
          :menu-open="openRuleMenuId === rule.id"
          @open="openEditor(rule)"
          @select="toggleSelection(rule)"
          @toggle-menu="openRuleMenuId = openRuleMenuId === rule.id ? null : rule.id; addMenuOpen = false; actionsMenuOpen = false; filterPanelOpen = false"
        >
            <template #menu>
              <KButton rounded inline clear class="gap-2" type="button" :disabled="busy || !rule.pattern.trim()" @click="closeRowMenu(); updateEnabled(rule, !rule.enabled)">{{ rule.enabled ? '停用' : '启用' }}</KButton>
              <KButton rounded inline clear class="gap-2" type="button" :disabled="busy" @click="closeRowMenu(); void replacementRulesFeature.removeReplacement(rule.id)">删除规则</KButton>
            </template>
        </ManagedListRow>
        <p v-if="!loadError && rules.length === 0" class="replacement-rule-empty">还没有规则</p>
        <p v-else-if="!loadError && !filteredRules.length" class="replacement-rule-empty">没有符合条件的规则</p>
      </div>
      <footer v-if="batchMode" class="replacement-selection-bar">
        <KButton rounded inline clear type="button" class="replacement-text-button gap-2" :disabled="busy || !filteredRules.length" @click="selectVisible()">{{ allVisibleSelected ? '取消全选' : '全选' }}</KButton>
        <KButton rounded inline clear type="button" class="replacement-text-button gap-2" :disabled="busy || !filteredRules.length" @click="selectVisible(true)">反选</KButton>
        <span class="replacement-selection-count">已选 {{ selectedRules.length }}</span>
        <KButton rounded inline clear type="button" class="replacement-text-button gap-2" :disabled="busy || !selectedRules.length" @click="exportSelectedRules">导出所选</KButton>
        <KButton rounded inline clear type="button" class="replacement-text-button replacement-delete-button k-color-brand-danger gap-2 " :disabled="busy || !selectedRules.length" @click="requestBulkDelete">删除</KButton>
      </footer>
    </div>
    <main v-if="draft" class="replacement-detail" data-back-page>
      <SubpageHeader class="replacement-editor-header col-span-full row-start-1" :title="draft.name || (editingRule ? '未命名规则' : '新净化规则')" back-label="返回规则列表" @back="requestCloseEditor" local-back>
<template #actions><label class="replacement-enabled-toggle"><span>启用</span><KToggle component="span" :checked="!!draft.enabled" @change="draft.enabled = ($event.target as HTMLInputElement).checked" :disabled="busy || !draft.pattern.trim()" /></label>
<div class="replacement-editor-actions">
          <KButton rounded inline type="button" class="replacement-primary-button replacement-mobile-save gap-2"
            :disabled="busy || upstreamRuleChanged || !draft.name.trim() || (draft.enabled && !draft.pattern.trim())"
            @click="saveEditor">保存</KButton>
        </div></template>
</SubpageHeader>
      <form class="replacement-editor-form" @submit.prevent="saveEditor">
        <section class="replacement-editor-section">
          <header><strong>规则</strong></header>
          <div class="replacement-editor-grid">
            <label class="replacement-field"><span>名称</span><input v-model="draft.name" maxlength="160" autocomplete="off" :disabled="busy" placeholder="规则名称"/></label>
            <label class="replacement-field"><span>作用范围</span><select v-model="draft.scope" :disabled="busy"><option v-for="scope in availableScopes" :key="scope.value" :value="scope.value">{{ scope.label }}</option></select></label>
          </div>
          <label class="replacement-field"><span>查找 / 正则</span><textarea v-model="draft.pattern" rows="4" spellcheck="false" :disabled="busy" placeholder="输入要查找的文字或正则表达式"/></label>
          <label class="replacement-field"><span>替换为</span><textarea v-model="draft.replacement" rows="4" spellcheck="false" :disabled="busy" placeholder="留空表示删除匹配内容"/></label>
          <div class="replacement-editor-options"><label><KToggle component="span" :checked="!!draft.isRegex" @change="draft.isRegex = ($event.target as HTMLInputElement).checked" :disabled="busy" /> 使用正则表达式</label></div>
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
          <KButton rounded inline tonal type="button" class="replacement-secondary-button gap-2" :disabled="busy" @click="reloadEditorRule">重新读取规则</KButton>
        </div>
        <p v-if="editorSaveError" class="replacement-validation">{{ editorSaveError }}</p>
        <div class="replacement-editor-bottom">
          <span></span>
          <KButton rounded inline tonal type="button" class="replacement-secondary-button gap-2"
            :disabled="busy || !editorDirty" @click="reloadEditorRule">还原</KButton>
          <KButton rounded inline type="submit" class="replacement-primary-button gap-2" :disabled="busy || upstreamRuleChanged || !draft.name.trim() || (draft.enabled && !draft.pattern.trim())">{{ editingRule ? '保存修改' : '添加规则' }}</KButton>
        </div>
      </form>
    </main>
    <div v-else class="hidden min-[861px]:grid place-content-center text-[var(--app-muted)]">选择左侧规则进行编辑，或添加新规则</div>
    <div v-if="replacementImportOpen" class="replacement-import-scrim" data-back-layer
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
          <KButton rounded inline type="submit" class="replacement-primary-button gap-2" :disabled="busy || replacementImportBusy || !replacementImportUrl.trim()">
            {{ replacementImportBusy ? '下载并导入中…' : '下载并导入' }}
          </KButton>
          <small>下载完成后会直接导入规则。</small>
        </form>
        <p v-if="replacementImportError" class="replacement-import-error">{{ replacementImportError }}</p>
        <div class="replacement-import-actions">
          <KButton rounded inline tonal type="button" class="replacement-secondary-button gap-2" :disabled="replacementImportBusy" data-back-action @click="replacementRulesFeature.cancelReplacementRulesUrlImport()">取消</KButton>
        </div>
      </section>
    </div>
    <div v-if="discardPromptOpen" class="replacement-discard-mask" data-back-layer
      @click.self="keepRuleEditor">
      <KDialog opened :backdrop="false" class="replacement-discard-dialog w-[min(420px,calc(100vw-32px))] max-h-[88dvh] overflow-auto">
  <template #title><h3 id="replacement-discard-title">放弃未保存的净化规则？</h3></template>
<p>当前规则的内容已经修改，{{ pendingNewRule ? '创建新规则前必须先放弃当前草稿。' : '返回或切换配置会丢弃这些草稿。' }}</p><template #buttons><KDialogButton type="button" data-back-action @click="keepRuleEditor">继续编辑</KDialogButton>
<KDialogButton type="button" @click="abandonRuleEditor" class="k-color-brand-danger" strong>放弃修改</KDialogButton></template>
</KDialog>
    </div>
  </section>
</template>

<style scoped>
@layer components {
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
  background:var(--app-background);
  color: var(--rr-ink);
}
.replacement-field input,.replacement-field select,.replacement-field textarea { min-width:0; min-height:38px; padding:0 10px; border:1px solid var(--rr-line); border-radius:8px; color:var(--rr-ink); background:var(--rr-bg); font:inherit; font-size:var(--ui-font-body); }
.replacement-menu { position:absolute; z-index:5; top:calc(100% + 5px); right:0; min-width:170px; display:grid; gap:2px; padding:5px; border:1px solid var(--rr-line); border-radius:10px; background:var(--rr-bg); box-shadow:0 10px 28px rgb(30 42 35 / 15%); }
.replacement-filter-panel { position:fixed; z-index:1000; box-sizing:border-box; display:grid; width:min(170px,calc(100vw - 16px)); max-height:calc(100vh - 16px); overflow-y:auto; padding:5px; border:1px solid var(--rr-line,var(--app-line)); border-radius:11px; background:var(--app-surface); box-shadow:0 12px 28px #22223322; }
.replacement-filter-panel button { display:block; width:100%; min-height:34px; padding:0 10px; border:0; border-radius:7px; color:var(--rr-ink,var(--app-text)); background:transparent; text-align:left; font:inherit; font-size:var(--ui-font-body); cursor:pointer; }
.replacement-menu button { min-height:34px; padding:0 9px; border:0; border-radius:6px; color:var(--rr-ink); background:transparent; text-align:left; font:inherit; font-size:var(--ui-font-body); cursor:pointer; }
.replacement-menu button:disabled { opacity:.45; cursor:default; }
.replacement-rule-list { min-width:0;min-height:0;flex:1 1 auto;display:grid;align-content:start;gap:3px;margin-right:-14px;padding:0 14px 0 0;overflow-y:auto;overscroll-behavior:contain; }
.replacement-page-message { padding:7px 14px; color:var(--rr-muted); font-size:var(--ui-font-caption); }
.replacement-page-message.error { color:var(--red); }
.replacement-text-button,.replacement-secondary-button,.replacement-primary-button { min-height:34px; padding:0 11px; border:1px solid var(--rr-line); border-radius:8px; color:var(--rr-ink); background:var(--rr-bg); font:inherit; font-size:var(--ui-font-body); cursor:pointer; }
.replacement-text-button { border-color:transparent; background:transparent; }
.replacement-primary-button { border-color:var(--rr-accent); color:#fff; background:var(--rr-accent); font-weight:650; }
.replacement-text-button:disabled,.replacement-secondary-button:disabled,.replacement-primary-button:disabled { opacity:.45; cursor:default; }
.replacement-editor-actions { display:flex; align-items:center; gap:5px; margin-left:auto; }
.replacement-field { min-width:0; display:grid; gap:6px; color:var(--rr-muted); font-size:var(--ui-font-caption); font-weight:600; }
.replacement-field input,.replacement-field select { width:100%; }
.replacement-field textarea { width:100%; min-height:100px; padding:9px 10px; resize:vertical; font-family:ui-monospace,SFMono-Regular,Consolas,monospace; line-height:1.55; }
.replacement-editor-options { display:flex; flex-wrap:wrap; gap:18px; color:var(--rr-ink); font-size:var(--ui-font-caption); }
.replacement-editor-options label { display:flex; align-items:center; gap:7px; }
.replacement-editor-options input { accent-color:var(--rr-accent); }
.replacement-validation { margin:0; color:var(--red); font-size:var(--ui-font-caption); }

.replacement-discard-mask{position:fixed;inset:0;z-index:120;display:grid;place-items:center;padding:16px;background:var(--app-scrim)}
.replacement-import-scrim{position:fixed;inset:0;z-index:2000;display:grid;place-items:center;padding:16px;background:var(--app-scrim)}
.replacement-import-dialog{box-sizing:border-box;width:min(420px,100%);max-height:min(82dvh,700px);overflow:auto;padding:22px;border:1px solid var(--rr-line);border-radius:14px;background:var(--app-surface);box-shadow:0 20px 40px #11112233}
.replacement-import-dialog h3{margin:0;font-size:var(--ui-font-heading)}
.replacement-import-url-form{display:grid;gap:9px;margin:10px 0 14px}
.replacement-import-url-form label{font-size:var(--ui-font-body);font-weight:650}
.replacement-import-url-form input{box-sizing:border-box;width:100%;min-height:39px;padding:9px 11px;border:1px solid var(--rr-line);border-radius:9px;background:var(--app-surface);color:var(--rr-ink);font:inherit;font-size:var(--ui-font-body)}
.replacement-import-url-form button{justify-self:start}
.replacement-import-url-form small{color:var(--rr-muted);font-size:var(--ui-font-caption);line-height:1.5}
.replacement-import-error{margin:9px 0 18px;color:var(--red);font-size:var(--ui-font-caption);line-height:1.5}
.replacement-import-actions{display:flex;justify-content:flex-end;gap:8px}
.replacement-discard-danger{background:#bc5964}
.replacement-editor-form input:disabled,.replacement-editor-form textarea:disabled,.replacement-editor-form select:disabled{cursor:not-allowed}

/* Content engine configuration workspace, matched to prototype editorPage('rules'). */
.replacement-add-wrap{position:relative;min-width:0}
.replacement-global-add {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  border-color: var(--rr-accent);
  background: var(--rr-accent);
  color: #fff;
  font-size: var(--ui-font-caption);
}
.replacement-global-add .prototype-icon{width:16px;height:16px}
.replacement-config-summary{display:flex;justify-content:space-between;gap:8px;padding:8px 5px 10px;margin:0}
.replacement-config-summary>div{display:flex;justify-content:space-between;gap:10px;width:100%}
.replacement-config-summary strong{color:var(--app-subtle);font-size:var(--ui-font-caption);font-weight:500}
.replacement-config-summary small{color:var(--app-subtle);font-size:var(--ui-font-caption)}
.replacement-selection-bar{position:static;z-index:4;display:flex;flex:none;align-items:center;gap:4px;flex-wrap:wrap;margin-top:auto;padding:8px 4px;border-top:1px solid var(--rr-line);background:var(--app-background)}
.replacement-selection-count{margin-left:auto;color:var(--rr-muted);font-size:var(--ui-font-caption);white-space:nowrap}
.replacement-delete-button{color:var(--app-danger)}
.replacement-rule-empty{align-self:center;justify-self:center;margin:auto;color:var(--rr-muted);font-size:var(--ui-font-caption);text-align:center}
.replacement-detail{min-width:0;min-height:0;overflow-y:auto;padding:28px 34px 48px;background:var(--app-background)}
.replacement-enabled-toggle{display:flex;align-items:center;gap:8px;color:var(--app-muted);font-size:var(--ui-font-caption);white-space:nowrap}
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
.replacement-editor-section>header strong{font-size:var(--ui-font-caption)}
.replacement-editor-grid{display:grid;grid-template-columns:1fr 1fr;gap:12px}
.replacement-editor-section .replacement-field{margin-bottom:14px;font-size:var(--ui-font-caption);font-weight:500;color:var(--app-muted)}
.replacement-editor-section .replacement-field input,
.replacement-editor-section .replacement-field select{box-sizing:border-box;height:36px;font-size:var(--ui-font-body)}
.replacement-editor-section .replacement-field textarea{min-height:84px;padding:9px;font-size:var(--ui-font-caption)}
.replacement-editor-options{margin:0 0 2px;font-size:var(--ui-font-caption)}
.replacement-test-result{min-height:42px;padding:12px;border:1px solid var(--rr-line);border-radius:10px;background:var(--app-panel-soft);font-size:var(--ui-font-caption);line-height:1.6;white-space:pre-wrap;overflow-wrap:anywhere}
.replacement-preview-note{margin:8px 0 0;color:var(--rr-muted);font-size:var(--ui-font-caption)}
.replacement-editor-bottom{display:grid;grid-template-columns:1fr auto auto;gap:8px;align-items:center;padding-top:16px;border-top:1px solid var(--rr-line)}
.replacement-editor-bottom .replacement-secondary-button{background:var(--app-panel-soft);border-color:transparent}
.replacement-editor-upstream{display:flex;align-items:center;gap:12px;justify-content:space-between;flex-wrap:wrap;padding:11px 12px;background:var(--app-danger-soft);border:1px solid var(--app-warning-line);border-radius:10px;color:var(--app-warning);font-size:var(--ui-font-caption);line-height:1.55}
.replacement-editor-upstream>span{flex:1;min-width:200px}
.replacement-master{display:flex;flex-direction:column;min-width:0;min-height:0;height:100%;max-height:none;overflow:hidden}
@media(min-width:861px){
  .replacement-rules-page{display:grid;grid-template-columns:300px minmax(0,1fr);grid-template-rows:auto minmax(0,1fr);align-items:start}
  .replacement-master{grid-row:2;grid-column:1;min-width:0;box-sizing:border-box;height:100%;max-height:none;overflow:hidden;padding:14px;border-right:1px solid var(--rr-line);background:var(--app-background)}
  .replacement-detail{grid-row:2;grid-column:2}
  .replacement-rules-page--empty .replacement-master{grid-column:1/-1;border-right:0}
}
@media(max-width:860px){
  .replacement-rules-page{display:flex;flex-direction:column}
  .replacement-master{height:auto;flex:1 1 auto;padding:16px var(--ui-page-gutter);min-width:0}
  .replacement-detail{position:fixed;inset:0;z-index:40;height:auto;padding:0 var(--ui-page-gutter) calc(32px + var(--safe-bottom))}
  .replacement-enabled-toggle{margin-left:auto}
  .replacement-mobile-save{display:inline-flex;align-items:center;min-height:33px;margin-left:3px}
  .replacement-editor-grid{grid-template-columns:1fr}
  .replacement-editor-bottom{grid-template-columns:1fr auto auto;gap:6px}
}
@media(max-width:480px){
  .replacement-editor-section .replacement-field input,
  .replacement-editor-section .replacement-field textarea,
  .replacement-editor-section .replacement-field select{font-size:var(--ui-font-body)}
}

}
</style>

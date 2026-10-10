<script setup lang="ts">
// 首页栏目编辑器负责自己的配置读取、表单草稿与保存。
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { onBeforeRouteLeave } from "vue-router";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { appBootstrap } from "../../api/core";
import { readResource } from "../../api/resources";
import router, { routeNames } from "../../router";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import BackButton from "../../ui/BackButton.vue";
import { useHomeConfig } from "./useHomeConfig";
import type { AppBootstrap, DiscoveryCategory, HomeConfigDocument, HomeSection, HomeTab, ResourceDescriptor, SourceDefinitionsResource, SourceMetadata } from "../../api/types";

function returnToHome(): void {
  void router.push({ name: routeNames.home });
}

const draft = ref<HomeConfigDocument>({ schemaVersion: 1, tabs: [] });
const sources = ref<SourceMetadata[]>([]);
const sourcesLoading = ref(true);
const sourcesError = ref("");
const restoreRevision = ref(0);
const activeTabId = ref("");
const expandedSectionId = ref<string | null>(null);
const draggingSectionId = ref<string | null>(null);
const dropSectionId = ref<string | null>(null);
const addingSection = ref(false);
const initialSnapshot = ref("");
const externalConfigChanged = ref(false);
const discardPromptOpen = ref(false);
let resolveNavigation: ((allow: boolean) => void) | null = null;
const newSectionSourceId = ref("");
const newSectionCategoryId = ref("");
const newSectionTitle = ref("");
let unlistenResourceUpdated: UnlistenFn | undefined;
let unlistenSourcesUpdated: UnlistenFn | undefined;
let unlistenAppStateUpdated: UnlistenFn | undefined;
let isUnmounted = false;
let acceptIncomingConfig = false;
let sourcesRequestRevision = 0;

const enabledNovelSources = computed(() => sources.value
  .filter((source) => source.enabled && source.isRss !== true)
  .slice()
  .sort((left, right) => left.name.localeCompare(right.name, "zh-CN")));
const homeConfigFeature = useHomeConfig({
  enabledSources: enabledNovelSources,
  getRestoreRevision: () => restoreRevision.value,
});
const {
  homeConfig,
  homeConfigLoading,
  homeConfigLoaded,
  homeConfigSaving,
  homeConfigError,
  homeEditorCategories,
  homeEditorLoadingSourceIds,
  homeEditorCategoryErrors,
  refreshHomeConfig,
  loadHomeEditorCategories,
  saveHomeConfigDraft,
  resetForRestore,
} = homeConfigFeature;
const busy = computed(() => sourcesLoading.value || homeConfigLoading.value || homeConfigSaving.value);
const categoryErrors = homeEditorCategoryErrors;
const orderedTabs = computed(() => [...draft.value.tabs].sort((left, right) => left.sortOrder - right.sortOrder));
const activeTab = computed(() => orderedTabs.value.find((tab) => tab.id === activeTabId.value) ?? orderedTabs.value[0]);
const orderedSections = computed(() => [...(activeTab.value?.sections ?? [])].sort((left, right) => left.sortOrder - right.sortOrder));
const categoriesForNewSection = computed(() => selectableCategories(newSectionSourceId.value));
const isDirty = computed(() => JSON.stringify(normalizeDraft(draft.value)) !== initialSnapshot.value);
onBeforeRouteLeave(() => {
  if (homeConfigSaving.value) return false;
  if (!isDirty.value) return true;
  return new Promise<boolean>((resolve) => {
    resolveNavigation?.(false);
    resolveNavigation = resolve;
    discardPromptOpen.value = true;
  });
});
onBeforeUnmount(() => {
  isUnmounted = true;
  restoreRevision.value += 1;
  sourcesRequestRevision += 1;
  resetForRestore();
  unlistenResourceUpdated?.();
  unlistenSourcesUpdated?.();
  unlistenAppStateUpdated?.();
  resolveNavigation?.(false);
});
function keepDraft(): void {
  discardPromptOpen.value = false;
  const resume = resolveNavigation;
  resolveNavigation = null;
  resume?.(false);
}
function discardDraft(): void {
  if (busy.value) return;
  discardPromptOpen.value = false;
  const resume = resolveNavigation;
  resolveNavigation = null;
  if (resume) resume(true);
  else reloadLatestConfig();
}
function requestDiscard(): void {
  if (busy.value) return;
  if (isDirty.value) discardPromptOpen.value = true;
  else reloadLatestConfig();
}

const validationErrors = computed(() => {
  const errors: string[] = [];
  const tabs = normalizeDraft(draft.value).tabs;
  if (tabs.length < 1 || tabs.length > 32) errors.push("主页需要保留 1 到 32 个标签页。");

  const tabNames = new Set<string>();
  const tabIds = new Set<string>();
  const sectionIds = new Set<string>();
  for (const tab of tabs) {
    const title = tab.title.trim();
    const key = title.toLocaleLowerCase();
    if (!title || [...title].length > 128) errors.push("每个标签页都需要名称，且名称不能超过 128 个字符。");
    if (tabNames.has(key)) errors.push("标签页名称不能重复。");
    if (tabIds.has(tab.id)) errors.push("标签页标识重复，请重新创建标签页。");
    tabNames.add(key);
    tabIds.add(tab.id);
    if (tab.sections.length > 64) errors.push(`“${title || "未命名标签页"}”最多可放置 64 个栏目。`);

    const infiniteGridCount = tab.sections.filter((section) => section.style === 2).length;
    if (infiniteGridCount > 1) errors.push(`“${title || "未命名标签页"}”最多只能有一个无限网格栏目。`);
    for (const section of tab.sections) {
      const sectionTitle = section.title.trim();
      if (!sectionTitle || [...sectionTitle].length > 128) errors.push("每个栏目都需要名称，且名称不能超过 128 个字符。");
      if (sectionIds.has(section.id)) errors.push("栏目标识重复，请重新创建栏目。");
      sectionIds.add(section.id);
      const source = sources.value.find((entry) => entry.id === section.sourceId);
      if (!source || !source.enabled || source.isRss === true) errors.push(`栏目“${sectionTitle || section.categoryName}”需要选择已启用的书籍来源。`);
      if (!section.categoryId.trim()) errors.push(`栏目“${sectionTitle || "未命名"}”需要选择一个分类。`);
      if (section.style < 0 || section.style > 3) errors.push(`栏目“${sectionTitle || "未命名"}”的展示样式无效。`);
    }
  }
  return [...new Set(errors)];
});

const canSave = computed(() => isDirty.value
  && homeConfigLoaded.value
  && !sourcesError.value
  && !externalConfigChanged.value
  && validationErrors.value.length === 0
  && !busy.value);

watch(
  homeConfig,
  (value) => {
    const copy = normalizeDraft(cloneDocument(value));
    const incomingSnapshot = JSON.stringify(copy);
    if (initialSnapshot.value && isDirty.value && !homeConfigSaving.value && !acceptIncomingConfig) {
      if (incomingSnapshot !== initialSnapshot.value) externalConfigChanged.value = true;
      return;
    }
    acceptIncomingConfig = false;
    draft.value = copy;
    initialSnapshot.value = incomingSnapshot;
    externalConfigChanged.value = false;
    if (!draft.value.tabs.some((tab) => tab.id === activeTabId.value)) activeTabId.value = draft.value.tabs[0]?.id ?? "";
  },
  { immediate: true, flush: "sync" },
);

watch(enabledNovelSources, (sources) => {
  if (!sources.some((source) => source.id === newSectionSourceId.value)) {
    newSectionSourceId.value = sources[0]?.id ?? "";
    newSectionCategoryId.value = "";
    newSectionTitle.value = "";
  }
}, { immediate: true });

watch(() => newSectionSourceId.value, (sourceId) => {
  newSectionCategoryId.value = "";
  newSectionTitle.value = "";
  if (sourceId) void loadHomeEditorCategories(sourceId);
});

async function reloadLatestConfig(): Promise<void> {
  if (busy.value) return;
  acceptIncomingConfig = true;
  const loaded = await refreshHomeConfig();
  if (!loaded) acceptIncomingConfig = false;
}

async function refreshSources(descriptor?: ResourceDescriptor, expectedRestoreRevision = restoreRevision.value): Promise<void> {
  const requestRevision = ++sourcesRequestRevision;
  sourcesLoading.value = true;
  try {
    const sourceDescriptor = descriptor ?? (await appBootstrap()).sources;
    const document = await readResource<SourceDefinitionsResource>(sourceDescriptor);
    if (isUnmounted || expectedRestoreRevision !== restoreRevision.value || requestRevision !== sourcesRequestRevision) return;
    sources.value = Array.isArray(document.sources) ? document.sources : [];
    sourcesError.value = "";
    sourcesLoading.value = false;
  } catch (error) {
    if (isUnmounted || expectedRestoreRevision !== restoreRevision.value || requestRevision !== sourcesRequestRevision) return;
    sourcesError.value = error instanceof Error ? error.message : String(error);
    sourcesLoading.value = false;
  }
}

function handleAppStateUpdated(snapshot: AppBootstrap): void {
  restoreRevision.value += 1;
  sourcesRequestRevision += 1;
  sources.value = [];
  sourcesLoading.value = true;
  sourcesError.value = "";
  acceptIncomingConfig = true;
  resetForRestore();
  void refreshSources(snapshot.sources, restoreRevision.value);
  void refreshHomeConfig().then((loaded) => {
    if (!loaded) acceptIncomingConfig = false;
  });
}

onMounted(async () => {
  const register = async <T,>(event: string, handler: (payload: T) => void | Promise<void>): Promise<UnlistenFn> => {
    const unlisten = await listen<T>(event, (entry) => { void handler(entry.payload); });
    if (isUnmounted) unlisten();
    return unlisten;
  };
  try {
    unlistenResourceUpdated = await register<{ kind: string; resource: ResourceDescriptor }>("resource-updated", (payload) => {
      if (payload.kind === "homeConfig" && !homeConfigSaving.value) return refreshHomeConfig(payload.resource).then(() => undefined);
    });
  } catch (error) {
    homeConfigError.value = `无法监听发现页配置更新：${error instanceof Error ? error.message : String(error)}`;
  }
  try {
    unlistenSourcesUpdated = await register<ResourceDescriptor>("sources-updated", (descriptor) => refreshSources(descriptor));
  } catch (error) {
    sourcesError.value = error instanceof Error ? error.message : String(error);
  }
  try {
    unlistenAppStateUpdated = await register<AppBootstrap>("app-state-updated", (snapshot) => handleAppStateUpdated(snapshot));
  } catch (error) {
    sourcesError.value = error instanceof Error ? error.message : String(error);
  }
  if (isUnmounted) return;
  await Promise.all([refreshSources(), refreshHomeConfig()]);
});

function cloneDocument(value: HomeConfigDocument): HomeConfigDocument {
  return JSON.parse(JSON.stringify(value)) as HomeConfigDocument;
}

function normalizeDraft(value: HomeConfigDocument): HomeConfigDocument {
  const tabs = [...(value.tabs ?? [])]
    .sort((left, right) => left.sortOrder - right.sortOrder)
    .map((tab, tabIndex) => ({
      ...tab,
      sortOrder: tabIndex,
      sections: [...(tab.sections ?? [])]
        .sort((left, right) => left.sortOrder - right.sortOrder)
        .map((section, sectionIndex) => ({ ...section, sortOrder: sectionIndex })),
    }));
  return { schemaVersion: value.schemaVersion || 1, tabs };
}

function nextId(prefix: "tab" | "section"): string {
  return `${prefix}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 11)}`;
}

function nextUniqueTabTitle(): string {
  const existing = new Set(draft.value.tabs.map((tab) => tab.title.trim().toLocaleLowerCase()));
  for (let number = 1; number <= 32; number += 1) {
    const candidate = number === 1 ? "新标签" : `新标签 ${number}`;
    if (!existing.has(candidate.toLocaleLowerCase())) return candidate;
  }
  return "新建标签页";
}

function addTab(): void {
  if (busy.value || draft.value.tabs.length >= 32) return;
  const tab: HomeTab = { id: nextId("tab"), title: nextUniqueTabTitle(), sortOrder: draft.value.tabs.length, sections: [] };
  draft.value.tabs.push(tab);
  activeTabId.value = tab.id;
}

function deleteActiveTab(): void {
  if (busy.value || !activeTab.value || draft.value.tabs.length <= 1) return;
  const index = orderedTabs.value.findIndex((tab) => tab.id === activeTab.value?.id);
  draft.value.tabs = orderedTabs.value.filter((tab) => tab.id !== activeTab.value?.id);
  draft.value.tabs.forEach((tab, order) => { tab.sortOrder = order; });
  activeTabId.value = draft.value.tabs[Math.max(0, index - 1)]?.id ?? draft.value.tabs[0]?.id ?? "";
}

function moveTab(tabId: string, direction: -1 | 1): void {
  if (busy.value) return;
  const tabs = orderedTabs.value;
  const index = tabs.findIndex((tab) => tab.id === tabId);
  const target = index + direction;
  if (index < 0 || target < 0 || target >= tabs.length) return;
  [tabs[index], tabs[target]] = [tabs[target], tabs[index]];
  draft.value.tabs = tabs.map((tab, sortOrder) => ({ ...tab, sortOrder }));
}

function selectableCategories(sourceId: string): DiscoveryCategory[] {
  return (homeEditorCategories.value[sourceId] ?? []).filter((category) => Boolean(category.categoryId));
}

function categoryPlaceholder(sourceId: string): string {
  if (!sourceId) return "先选择书籍来源";
  if (homeEditorCategoryErrors.value[sourceId]) return "读取失败，请重试";
  if (homeEditorLoadingSourceIds.value.includes(sourceId)) return "正在读取分类…";
  if (!categoriesLoaded(sourceId)) return "等待读取分类";
  return selectableCategories(sourceId).length ? "选择分类" : "没有可用分类";
}

function categoriesLoaded(sourceId: string): boolean {
  return Object.prototype.hasOwnProperty.call(homeEditorCategories.value, sourceId);
}

function selectedCategory(sourceId: string, categoryId: string): DiscoveryCategory | undefined {
  return selectableCategories(sourceId).find((category) => category.categoryId === categoryId);
}

function sourceName(sourceId: string): string {
  return sources.value.find((source) => source.id === sourceId)?.name ?? "已保存来源";
}

function categoryChoices(section: HomeSection): DiscoveryCategory[] {
  const choices = selectableCategories(section.sourceId);
  if (section.categoryId && !choices.some((category) => category.categoryId === section.categoryId)) {
    return [{ categoryId: section.categoryId, title: `${section.categoryName}（已保存）` }, ...choices];
  }
  return choices;
}

function changeSectionSource(section: HomeSection, sourceId: string): void {
  if (busy.value) return;
  section.sourceId = sourceId;
  section.sourceName = sourceName(sourceId);
  section.categoryId = "";
  section.categoryName = "";
  void loadHomeEditorCategories(sourceId);
}

function changeSectionCategory(section: HomeSection, categoryId: string): void {
  section.categoryId = categoryId;
  section.categoryName = selectedCategory(section.sourceId, categoryId)?.title ?? section.categoryName;
}

function updateSectionStyle(section: HomeSection, event: Event): void {
  const value = Number((event.target as HTMLSelectElement).value);
  if (value >= 0 && value <= 3) section.style = value as HomeSection["style"];
}

function addSection(): void {
  const tab = activeTab.value;
  const category = selectedCategory(newSectionSourceId.value, newSectionCategoryId.value);
  if (busy.value || !tab || !category?.categoryId || tab.sections.length >= 64) return;
  const name = newSectionTitle.value.trim() || category.title;
  tab.sections.push({
    id: nextId("section"),
    title: name,
    sourceId: newSectionSourceId.value,
    sourceName: sourceName(newSectionSourceId.value),
    categoryId: category.categoryId,
    categoryName: category.title,
    style: 0,
    sortOrder: tab.sections.length,
    coverVideo: false,
  });
  newSectionCategoryId.value = "";
  newSectionTitle.value = "";
  addingSection.value = false;
}

function removeSection(sectionId: string): void {
  if (busy.value || !activeTab.value) return;
  activeTab.value.sections = activeTab.value.sections.filter((section) => section.id !== sectionId);
  activeTab.value.sections.forEach((section, sortOrder) => { section.sortOrder = sortOrder; });
}

function moveSection(sectionId: string, direction: -1 | 1): void {
  if (busy.value || !activeTab.value) return;
  const sections = orderedSections.value;
  const index = sections.findIndex((section) => section.id === sectionId);
  const target = index + direction;
  if (index < 0 || target < 0 || target >= sections.length) return;
  [sections[index], sections[target]] = [sections[target], sections[index]];
  activeTab.value.sections = sections.map((section, sortOrder) => ({ ...section, sortOrder }));
}

/** Native drag only starts on the grip; edit inputs remain ordinary controls.
 *  Mobile / keyboard ordering keeps using the accessible move buttons.
 */
function beginSectionDrag(event: DragEvent, sectionId: string): void {
  if (busy.value || !event.dataTransfer) { event.preventDefault(); return; }
  draggingSectionId.value = sectionId;
  dropSectionId.value = null;
  event.dataTransfer.effectAllowed = "move";
  event.dataTransfer.setData("text/plain", sectionId);
}
function trackSectionDrop(event: DragEvent, targetId: string): void {
  if (!draggingSectionId.value || busy.value) return;
  event.preventDefault();
  if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
  dropSectionId.value = targetId === draggingSectionId.value ? null : targetId;
}
function finishSectionDrag(): void {
  draggingSectionId.value = null;
  dropSectionId.value = null;
}
function applySectionDrop(event: DragEvent, targetId: string): void {
  if (!draggingSectionId.value) return;
  event.preventDefault();
  const movedId = draggingSectionId.value;
  if (!movedId || busy.value || !activeTab.value || movedId === targetId) { finishSectionDrag(); return; }
  const sections = orderedSections.value;
  const origin = sections.findIndex((section) => section.id === movedId);
  const target = sections.findIndex((section) => section.id === targetId);
  if (origin < 0 || target < 0) { finishSectionDrag(); return; }
  const [moved] = sections.splice(origin, 1);
  if (!moved) { finishSectionDrag(); return; }
  sections.splice(target, 0, moved);
  activeTab.value.sections = sections.map((section, sortOrder) => ({ ...section, sortOrder }));
  finishSectionDrag();
}

async function submit(): Promise<void> {
  if (!canSave.value) return;
  const currentSnapshot = JSON.stringify(normalizeDraft(cloneDocument(homeConfig.value)));
  if (currentSnapshot !== initialSnapshot.value) {
    externalConfigChanged.value = true;
    return;
  }
  const saved = await saveHomeConfigDraft(normalizeDraft(cloneDocument(draft.value)));
  if (!saved) return;
  const accepted = normalizeDraft(cloneDocument(homeConfig.value));
  draft.value = accepted;
  initialSnapshot.value = JSON.stringify(accepted);
  externalConfigChanged.value = false;
  if (!draft.value.tabs.some((tab) => tab.id === activeTabId.value)) activeTabId.value = draft.value.tabs[0]?.id ?? "";
}
</script>

<template>
  <section class="home-config-editor my-settings-subpage">
    <header class="home-config-editor__topbar">
      <BackButton label="返回发现" @click="returnToHome" />
      <div class="home-config-editor__top-title">
        <strong>发现页配置</strong>
      </div>
      <button type="button" class="home-config-editor__button home-config-editor__button--primary home-config-editor__top-save"
        :disabled="!canSave" @click="submit">{{ homeConfigSaving ? '保存中…' : '保存' }}</button>
    </header>
    <div class="home-config-editor__body">
      <p v-if="homeConfigLoading" class="home-config-editor__load-state">正在读取发现页配置…</p>
      <div v-if="sourcesError" class="home-config-editor__validation" role="alert">
        读取书源失败：{{ sourcesError }}
        <button type="button" class="home-config-editor__button home-config-editor__button--secondary" :disabled="sourcesLoading" @click="refreshSources()">重新读取书源</button>
      </div>
      <div v-if="homeConfigError" class="home-config-editor__validation" role="alert">
        {{ homeConfigError }}
        <button v-if="!homeConfigLoaded" type="button" class="home-config-editor__button home-config-editor__button--secondary" :disabled="busy" @click="reloadLatestConfig">重新读取配置</button>
      </div>
      <div class="home-config-editor__tabs-row">
        <div class="home-config-editor__tabs">
          <button
            v-for="tab in orderedTabs"
            :key="tab.id"
            type="button"
            :class="{ active: activeTab?.id === tab.id }"
            @click="activeTabId = tab.id"
          >{{ tab.title }}</button>
          <button type="button" class="home-config-editor__tab-add" :disabled="busy || draft.tabs.length >= 32"
            @click="addTab"><PrototypeIcon name="plus"/><span>添加</span></button>
        </div>
      </div>

      <details v-if="activeTab" class="home-config-editor__tab-details">
        <summary><PrototypeIcon name="tune"/> 编辑「{{ activeTab.title }}」标签 <PrototypeIcon name="right"/></summary>
        <section class="home-config-editor__tab-settings">
        <label class="home-config-editor__field home-config-editor__tab-name">
          <span>标签页名称</span>
          <input v-model="activeTab.title" maxlength="128" :disabled="busy" />
        </label>
        <div class="home-config-editor__tab-actions">
          <button type="button" class="home-config-editor__button home-config-editor__button--secondary" :disabled="busy || orderedTabs.findIndex((tab) => tab.id === activeTab?.id) === 0" @click="moveTab(activeTab.id, -1)">上移</button>
          <button type="button" class="home-config-editor__button home-config-editor__button--secondary" :disabled="busy || orderedTabs.findIndex((tab) => tab.id === activeTab?.id) === orderedTabs.length - 1" @click="moveTab(activeTab.id, 1)">下移</button>
          <button type="button" class="home-config-editor__button home-config-editor__button--danger" :disabled="busy || draft.tabs.length <= 1" @click="deleteActiveTab">删除标签页</button>
        </div>
        </section>
      </details>

      <section class="home-config-editor__sections">
        <div class="home-config-editor__section-heading">
          <span>已配置 {{ activeTab?.sections.length ?? 0 }} / 64 个栏目</span>
        </div>

        <div v-if="!orderedSections.length" class="home-config-editor__empty">这个标签页还没有栏目。</div>

        <article v-for="(section, index) in orderedSections" :key="section.id" class="home-config-editor__section-card"
          :class="{ 'is-drop-target': dropSectionId === section.id }"
          @dragover="trackSectionDrop($event, section.id)" @drop="applySectionDrop($event, section.id)">
          <div class="home-config-editor__section-title-row">
            <span class="home-config-editor__order-grip" :draggable="!busy"
              title="拖动排序；移动端可使用右侧上下按钮"
              @dragstart="beginSectionDrag($event, section.id)" @dragend="finishSectionDrag"><i></i><i></i><i></i></span>
            <label class="home-config-editor__field home-config-editor__section-title">
              <input v-model="section.title" maxlength="128" :disabled="busy"
                />
              <small :title="section.sourceName + ' · ' + section.categoryName">{{ section.sourceName }} · {{ section.categoryName }}</small>
            </label>
            <label class="home-config-editor__compact-style">
              <select :value="section.style" :disabled="busy"
@change="updateSectionStyle(section, $event)">
                <option :value="0">横向封面</option><option :value="1">排行列表</option><option :value="2">无限网格</option><option :value="3">四列卡片</option>
              </select>
            </label>
            <button type="button" class="home-config-editor__expand" :class="{ 'is-open': expandedSectionId === section.id }"
              @click="expandedSectionId = expandedSectionId === section.id ? null : section.id">
              <PrototypeIcon name="more"/>
            </button>
          </div>

          <div v-if="expandedSectionId === section.id" class="home-config-editor__section-fields">
            <label class="home-config-editor__field">
              <span>书籍来源</span>
              <select :value="section.sourceId" :disabled="busy" @change="changeSectionSource(section, ($event.target as HTMLSelectElement).value)">
                <option v-for="source in enabledNovelSources" :key="source.id" :value="source.id">{{ source.name }}</option>
              </select>
            </label>
            <label class="home-config-editor__field">
              <span>分类</span>
              <select :value="section.categoryId" :disabled="busy || !section.sourceId" @focus="loadHomeEditorCategories(section.sourceId)" @change="changeSectionCategory(section, ($event.target as HTMLSelectElement).value)">
                <option value="" disabled>{{ categoryPlaceholder(section.sourceId) }}</option>
                <option v-for="category in categoryChoices(section)" :key="category.categoryId" :value="category.categoryId">{{ category.title }}</option>
              </select>
              <small v-if="categoryErrors[section.sourceId]" class="home-config-editor__category-error">{{ categoryErrors[section.sourceId] }} <button type="button" :disabled="busy" @click="loadHomeEditorCategories(section.sourceId)">重新读取</button></small>
              <small v-else-if="!categoriesLoaded(section.sourceId)">聚焦分类框可读取此来源的其他分类。</small>
            </label>
            <label class="home-config-editor__checkbox-field">
              <input v-model="section.coverVideo" type="checkbox" :disabled="busy" />
              <span>展示封面动画</span>
            </label>
            <div class="home-config-editor__section-actions">
              <button type="button" class="home-config-editor__icon-button"
                :disabled="busy || index === 0" @click="moveSection(section.id, -1)">
                <PrototypeIcon class="section-arrow-up" name="right"/>
              </button>
              <button type="button" class="home-config-editor__icon-button"
                :disabled="busy || index === orderedSections.length - 1" @click="moveSection(section.id, 1)">
                <PrototypeIcon class="section-arrow-down" name="right"/>
              </button>
              <button type="button" class="home-config-editor__button home-config-editor__button--danger"
                :disabled="busy" @click="removeSection(section.id)">
                移除栏目
              </button>
            </div>
          </div>
        </article>

        <section class="home-config-editor__add-section">
          <button type="button" class="home-config-editor__add-toggle"
            :disabled="busy || !enabledNovelSources.length"
            @click="addingSection = !addingSection">{{ addingSection ? '收起添加栏目' : '+ 添加栏目' }}</button>
          <div v-if="addingSection" class="home-config-editor__add-grid">
            <label class="home-config-editor__field">
              <span>书籍来源</span>
              <select v-model="newSectionSourceId" :disabled="busy || !enabledNovelSources.length">
                <option value="" disabled>选择书籍来源</option>
                <option v-for="source in enabledNovelSources" :key="source.id" :value="source.id">{{ source.name }}</option>
              </select>
            </label>
            <label class="home-config-editor__field">
              <span>分类</span>
              <select v-model="newSectionCategoryId" :disabled="busy || !newSectionSourceId">
                <option value="" disabled>{{ categoryPlaceholder(newSectionSourceId) }}</option>
                <option v-for="category in categoriesForNewSection" :key="category.categoryId" :value="category.categoryId">{{ category.title }}</option>
              </select>
            </label>
            <label class="home-config-editor__field">
              <span>栏目名称（可选）</span>
              <input v-model="newSectionTitle" maxlength="128" :disabled="busy" placeholder="默认使用分类名称" />
            </label>
            <button type="button" class="home-config-editor__button home-config-editor__button--primary" :disabled="busy || !activeTab || activeTab.sections.length >= 64 || !selectedCategory(newSectionSourceId, newSectionCategoryId)" @click="addSection">添加栏目</button>
          </div>
          <p v-if="categoryErrors[newSectionSourceId]" class="home-config-editor__category-error">{{ categoryErrors[newSectionSourceId] }} <button type="button" :disabled="busy || !newSectionSourceId" @click="loadHomeEditorCategories(newSectionSourceId)">重新读取分类</button></p>
          <p v-if="!enabledNovelSources.length" class="home-config-editor__hint">没有已启用的书籍来源可供选择。</p>
        </section>
      </section>

      <div v-if="externalConfigChanged" class="home-config-editor__validation">
        发现页配置已在其他操作中更新。当前草稿仍保留，为防止覆盖最新配置，已禁止保存。请放弃草稿并重新读取后再编辑。
        <button type="button" class="home-config-editor__button home-config-editor__button--secondary" :disabled="busy" @click="reloadLatestConfig">放弃草稿并读取最新配置</button>
      </div>
      <div v-if="validationErrors.length" class="home-config-editor__validation">
        <strong>保存前请处理以下问题</strong>
        <ul><li v-for="error in validationErrors" :key="error">{{ error }}</li></ul>
      </div>
    </div>

    <footer class="home-config-editor__footer">
      <span class="home-config-editor__save-hint">{{ externalConfigChanged ? '配置已更新，需要重新读取' : isDirty ? '有尚未保存的修改' : '所有修改已保存' }}</span>
      <button type="button" class="home-config-editor__button home-config-editor__button--secondary"
        :disabled="busy || !isDirty" @click="requestDiscard">放弃修改</button>
    </footer>
    <div v-if="discardPromptOpen" class="home-config-editor__discard-mask">
      <section class="home-config-editor__discard-dialog">
        <h3 id="home-config-discard-title">放弃发现页配置修改？</h3>
        <p>标签、栏目、排序和展示样式的未保存修改都会丢失。</p>
        <footer><button type="button" class="home-config-editor__button home-config-editor__button--secondary" @click="keepDraft">继续编辑</button>
          <button type="button" class="home-config-editor__button home-config-editor__button--danger" @click="discardDraft">放弃修改</button></footer>
      </section>
    </div>
  </section>
</template>

<style scoped>
.home-config-editor{
  --hc-accent:var(--app-accent);
  --hc-line:var(--app-line);
  --hc-muted:#999da5;
  box-sizing:border-box;
  width:100%;
  min-width:0;
  min-height:100%;
  background:#f6f7f9;
  color:#343640;
}
.home-config-editor__topbar{
  position:sticky;
  top:0;
  z-index:35;
  box-sizing:border-box;
  display:grid;
  grid-template-columns:44px minmax(0,1fr) auto;
  align-items:center;
  gap:10px;
  height:64px;
  padding:0 max(18px,calc((100% - 1180px)/2));
  border-bottom:1px solid var(--hc-line);
  background:#f6f7f9ee;
  backdrop-filter:blur(18px);
}
.home-config-editor__top-title{min-width:0}
.home-config-editor__top-title strong{font-size:14px;font-weight:750;color:#242631}
.home-config-editor__button{
  display:inline-flex;align-items:center;justify-content:center;gap:6px;
  min-height:34px;padding:0 12px;
  border:1px solid transparent;border-radius:9px;
  font-size:10px;font-weight:650;cursor:pointer;
}
.home-config-editor__button:disabled,.home-config-editor__icon-button:disabled{opacity:.45;cursor:default}
.home-config-editor__button--primary{background:var(--hc-accent);color:#fff}
.home-config-editor__button--secondary{border-color:var(--hc-line);background:#fff;color:#606571}
.home-config-editor__button--danger{background:#fff0f1;color:#ae535b}
.home-config-editor__top-save{min-width:60px}
.home-config-editor__body{
  box-sizing:border-box;width:min(820px,calc(100% - 36px));
  margin:0 auto;padding:24px 0 26px;
}
.home-config-editor__tabs-row{display:block;margin:0 0 12px}
.home-config-editor__tabs{
  display:flex;align-items:center;gap:6px;
  min-width:0;max-width:100%;padding:0;
  overflow-x:auto;scrollbar-width:thin;
}
.home-config-editor__tabs button{
  display:inline-flex;align-items:center;justify-content:center;
  flex:none;box-sizing:border-box;height:29px;min-height:29px;
  padding:0 10px;border:0;border-radius:999px;
  background:#eceef2;color:#696e77;font-size:9px;
  font-weight:600;white-space:nowrap;cursor:pointer;
}
.home-config-editor__tabs button.active{background:#202126;color:#fff;font-weight:750}
.home-config-editor__tabs .home-config-editor__tab-add{
  gap:4px;border:1px dashed #c8c2dd;background:#faf9fd;color:var(--hc-accent);
}
.home-config-editor__tab-add :deep(svg){width:13px;height:13px}
.home-config-editor__tab-details{
  margin:0 0 13px;border-top:0;border-bottom:1px solid var(--hc-line);
}
.home-config-editor__tab-details>summary{
  display:flex;align-items:center;gap:8px;
  min-height:34px;padding:0 4px;list-style:none;
  color:#7b7e89;font-size:10px;cursor:pointer;
}
.home-config-editor__tab-details>summary::-webkit-details-marker{display:none}
.home-config-editor__tab-details>summary>.prototype-icon{width:15px;height:15px;color:#7c6bb4}
.home-config-editor__tab-details>summary>.prototype-icon:last-child{margin-left:auto;color:#a3a6b0}
.home-config-editor__tab-details[open]>summary>.prototype-icon:last-child{transform:rotate(90deg)}
.home-config-editor__tab-settings{
  display:flex;align-items:end;gap:10px;
  padding:9px 4px 14px;min-width:0;
}
.home-config-editor__tab-name{flex:1}
.home-config-editor__tab-actions{display:flex;align-items:center;gap:6px}
.home-config-editor__field{display:flex;flex-direction:column;gap:4px;min-width:0;color:#777c85;font-size:9px}
.home-config-editor__field>span{font-weight:600}
.home-config-editor__field input,.home-config-editor__field select{
  box-sizing:border-box;width:100%;min-width:0;height:36px;min-height:36px;
  padding:0 9px;border:1px solid var(--hc-line);border-radius:8px;
  background:#fff;color:#343640;font:inherit;font-size:10px;
}
.home-config-editor__field small{color:var(--hc-muted);font-size:8px;line-height:1.45}
.home-config-editor__category-error{color:#b0525d!important;font-size:9px}
.home-config-editor__category-error button{
  margin-left:4px;border:0;background:transparent;color:var(--hc-accent);
  font-size:9px;text-decoration:underline;cursor:pointer;
}
.home-config-editor__section-heading{
  display:flex;align-items:center;justify-content:space-between;
  gap:10px;margin:3px 0 8px;color:var(--hc-muted);font-size:9px;
}
.home-config-editor__section-heading>span{white-space:nowrap}
.home-config-editor__empty{
  display:grid;place-items:center;min-height:90px;
  padding:16px 0;color:#999da5;font-size:10px;text-align:center;
}
.home-config-editor__sections{min-width:0}
.home-config-editor__section-card{
  box-sizing:border-box;min-height:58px;
  margin:0 0 8px;padding:10px;
  border:1px solid var(--hc-line);border-radius:11px;background:#fff;
}
.home-config-editor__section-card.is-drop-target{border-color:var(--hc-accent);box-shadow:0 0 0 2px #6757d620}
.home-config-editor__section-title-row{
  display:grid;
  grid-template-columns:20px minmax(0,1fr) 130px 36px;
  align-items:center;gap:9px;min-height:36px;
}
.home-config-editor__order-grip{
  display:grid;grid-template-columns:repeat(2,3px);
  justify-content:center;align-content:center;gap:3px;
  height:34px;cursor:grab;user-select:none;
}
.home-config-editor__order-grip:active{cursor:grabbing}
.home-config-editor__order-grip i{
  width:3px;height:13px;border-left:3px dotted #afb3bb;
}
.home-config-editor__section-title{display:flex;flex-direction:column;gap:2px;min-width:0}
.home-config-editor__section-title input{
  box-sizing:border-box;width:100%;height:20px;min-height:20px;
  padding:0;border:0;border-radius:0;outline-offset:2px;
  background:transparent;color:#30323b;font-size:10px;font-weight:700;
}
.home-config-editor__section-title small{
  min-width:0;overflow:hidden;white-space:nowrap;text-overflow:ellipsis;
  color:#999da5;font-size:7px;font-weight:400;
}
.home-config-editor__compact-style select{
  box-sizing:border-box;width:100%;height:31px;min-height:31px;
  padding:0 7px;border:1px solid var(--hc-line);border-radius:8px;
  background:#fff;color:#545862;font-size:9px;
}
.home-config-editor__expand{
  display:grid;place-items:center;width:36px;height:36px;
  border:0;border-radius:9px;background:transparent;color:#777d8a;cursor:pointer;
}
.home-config-editor__expand:hover,.home-config-editor__expand.is-open{background:#f1edff;color:var(--hc-accent)}
.home-config-editor__expand :deep(svg){width:18px;height:18px}
.home-config-editor__section-fields{
  display:grid;grid-template-columns:1fr 1fr auto;
  align-items:end;gap:10px;margin-top:9px;padding:13px 5px 3px;
  border-top:1px solid var(--hc-line);
}
.home-config-editor__checkbox-field{
  display:flex;align-items:center;gap:6px;min-height:36px;
  color:#777b85;font-size:9px;white-space:nowrap;
}
.home-config-editor__checkbox-field input{width:16px;height:16px;accent-color:var(--hc-accent)}
.home-config-editor__section-actions{
  display:flex;justify-content:flex-end;align-items:center;
  grid-column:1/-1;gap:5px;
}
.home-config-editor__icon-button{
  display:grid;place-items:center;width:32px;height:32px;padding:0;
  border:1px solid var(--hc-line);border-radius:8px;
  background:#fff;color:#8175a2;cursor:pointer;
}
.home-config-editor__icon-button:hover:not(:disabled){background:#f2efff}
.home-config-editor__icon-button :deep(svg){width:16px;height:16px}
.home-config-editor__icon-button .section-arrow-up{transform:rotate(-90deg)}
.home-config-editor__icon-button .section-arrow-down{transform:rotate(90deg)}
.home-config-editor__add-section{margin-top:10px}
.home-config-editor__add-toggle{
  box-sizing:border-box;width:100%;min-height:41px;
  border:1px dashed #d6cdec;border-radius:11px;
  background:#faf9ff;color:var(--hc-accent);
  text-align:center;font-size:10px;font-weight:700;cursor:pointer;
}
.home-config-editor__add-toggle:disabled{opacity:.5;cursor:default}
.home-config-editor__add-grid{
  display:grid;grid-template-columns:1fr 1fr 1.1fr auto;
  align-items:end;gap:10px;margin-top:12px;padding:11px;
  border:1px solid var(--hc-line);border-radius:10px;background:#fff;
}
.home-config-editor__hint{margin:9px 0;color:var(--hc-muted);font-size:10px}
.home-config-editor__load-state{margin:0 0 12px;color:var(--hc-muted);font-size:10px}
.home-config-editor__validation{
  margin-top:14px;padding:11px 12px;border:1px solid #edcfc9;
  border-radius:9px;background:#fff4f3;color:#89564d;
  font-size:10px;line-height:1.6;
}
.home-config-editor__validation ul{margin:7px 0 0;padding-left:17px}
.home-config-editor__footer{
  display:flex;align-items:center;gap:10px;
  box-sizing:border-box;width:min(820px,calc(100% - 36px));
  margin:0 auto;padding:11px 0 20px;border-top:1px solid var(--hc-line);
}
.home-config-editor__save-hint{flex:1;color:var(--hc-muted);font-size:9px}
.home-config-editor__discard-mask{
  position:fixed;inset:0;z-index:120;
  display:grid;place-items:center;padding:16px;background:#191b2b88;
}
.home-config-editor__discard-dialog{
  box-sizing:border-box;width:min(430px,100%);padding:22px;
  border:1px solid var(--hc-line);border-radius:14px;
  background:#fff;box-shadow:0 16px 44px #11142822;
}
.home-config-editor__discard-dialog h3{margin:0;color:#343640;font-size:15px}
.home-config-editor__discard-dialog p{margin:10px 0;color:#888c97;font-size:11px;line-height:1.65}
.home-config-editor__discard-dialog footer{display:flex;justify-content:flex-end;gap:8px;margin-top:17px}
@media(max-width:860px){
  .home-config-editor__topbar{padding:0 14px}
  .home-config-editor__body{padding:18px 0 22px}
  .home-config-editor__section-title-row{grid-template-columns:18px minmax(0,1fr) 100px 32px;gap:8px}
  .home-config-editor__expand{width:32px}
  .home-config-editor__section-fields{grid-template-columns:repeat(2,minmax(0,1fr))}
  .home-config-editor__checkbox-field{grid-column:1/-1}
}
@media(max-width:560px){
  .home-config-editor__body,.home-config-editor__footer{width:calc(100% - 28px)}
  .home-config-editor__topbar{padding:0 12px}
  .home-config-editor__section-title-row{grid-template-columns:18px minmax(0,1fr) 32px}
  .home-config-editor__compact-style{grid-column:2;grid-row:2}
  .home-config-editor__expand{grid-column:3;grid-row:1/3}
  .home-config-editor__section-fields{grid-template-columns:minmax(0,1fr)}
  .home-config-editor__checkbox-field{grid-column:1}
  .home-config-editor__tab-settings{flex-direction:column;align-items:stretch}
  .home-config-editor__tab-actions{flex-wrap:wrap}
  .home-config-editor__add-grid{grid-template-columns:minmax(0,1fr)}
}
</style>

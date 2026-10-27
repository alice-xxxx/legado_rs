<script setup lang="ts">
import { Toggle as KToggle, Button as KButton } from "konsta/vue";
import SubpageHeader from "../../ui/SubpageHeader.vue";
// 首页栏目直接编辑全局设置中的 home 配置。
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { sources } from "../sources/sourcesStore";
import router, { routeNames } from "../../router";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import { useHomeConfig } from "./useHomeConfig";
import type { AppBootstrap, DiscoveryCategory, HomeConfigDocument, HomeSection, HomeTab } from "../../api/types";

function returnFromHomeConfig(): void {
  if (router.options.history.state.back) void router.back();
  else void router.replace({ name: routeNames.settingsOther });
}

const restoreRevision = ref(0);
const activeTabId = ref("");
const expandedSectionId = ref<string | null>(null);
const draggingSectionId = ref<string | null>(null);
const dropSectionId = ref<string | null>(null);
const addingSection = ref(false);
const newSectionSourceId = ref("");
const newSectionCategoryId = ref("");
const newSectionTitle = ref("");
let unlistenAppStateUpdated: UnlistenFn | undefined;
let isUnmounted = false;

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
  homeEditorCategories,
  homeEditorLoadingSourceIds,
  homeEditorCategoryErrors,
  loadHomeEditorCategories,
  resetForRestore,
} = homeConfigFeature;
const draft = homeConfig;
const categoryErrors = homeEditorCategoryErrors;
const orderedTabs = computed(() => [...draft.value.tabs].sort((left, right) => left.sortOrder - right.sortOrder));
const activeTab = computed(() => orderedTabs.value.find((tab) => tab.id === activeTabId.value) ?? orderedTabs.value[0]);
const orderedSections = computed(() => [...(activeTab.value?.sections ?? [])].sort((left, right) => left.sortOrder - right.sortOrder));
const categoriesForNewSection = computed(() => selectableCategories(newSectionSourceId.value));
onBeforeUnmount(() => {
  isUnmounted = true;
  restoreRevision.value += 1;
  resetForRestore();
  unlistenAppStateUpdated?.();
});

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
      if (!source || !source.enabled || source.isRss === true) errors.push(`栏目“${sectionTitle || section.categoryId}”需要选择已启用的书籍来源。`);
      if (!section.categoryId.trim()) errors.push(`栏目“${sectionTitle || "未命名"}”需要选择一个分类。`);
      if (section.style < 0 || section.style > 3) errors.push(`栏目“${sectionTitle || "未命名"}”的展示样式无效。`);
    }
  }
  return [...new Set(errors)];
});

watch(
  () => homeConfig.value.tabs,
  (tabs) => {
    if (!draft.value.tabs.some((tab) => tab.id === activeTabId.value)) activeTabId.value = draft.value.tabs[0]?.id ?? "";
    for (const sourceId of new Set(tabs.flatMap((tab) => tab.sections.map((section) => section.sourceId)))) {
      if (enabledNovelSources.value.some((source) => source.id === sourceId)) void loadHomeEditorCategories(sourceId);
    }
  },
  { immediate: true, deep: true, flush: "sync" },
);

watch(enabledNovelSources, (sources) => {
  if (!sources.some((source) => source.id === newSectionSourceId.value)) {
    newSectionSourceId.value = sources[0]?.id ?? "";
    newSectionCategoryId.value = "";
    newSectionTitle.value = "";
  }
  for (const sourceId of new Set(homeConfig.value.tabs.flatMap((tab) => tab.sections.map((section) => section.sourceId)))) {
    if (sources.some((source) => source.id === sourceId)) void loadHomeEditorCategories(sourceId);
  }
}, { immediate: true });

watch(() => newSectionSourceId.value, (sourceId) => {
  newSectionCategoryId.value = "";
  newSectionTitle.value = "";
  if (sourceId) void loadHomeEditorCategories(sourceId);
});

function handleAppStateUpdated(): void {
  restoreRevision.value += 1;
  resetForRestore();
}

onMounted(async () => {
  unlistenAppStateUpdated = await listen<AppBootstrap>("app-state-updated", () => handleAppStateUpdated());
  if (isUnmounted) unlistenAppStateUpdated();
});

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
  return { tabs };
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
  if (draft.value.tabs.length >= 32) return;
  const tab: HomeTab = { id: nextId("tab"), title: nextUniqueTabTitle(), sortOrder: draft.value.tabs.length, sections: [] };
  draft.value.tabs.push(tab);
  activeTabId.value = tab.id;
}

function deleteActiveTab(): void {
  if (!activeTab.value || draft.value.tabs.length <= 1) return;
  const index = orderedTabs.value.findIndex((tab) => tab.id === activeTab.value?.id);
  draft.value.tabs = orderedTabs.value.filter((tab) => tab.id !== activeTab.value?.id);
  draft.value.tabs.forEach((tab, order) => { tab.sortOrder = order; });
  activeTabId.value = draft.value.tabs[Math.max(0, index - 1)]?.id ?? draft.value.tabs[0]?.id ?? "";
}

function moveTab(tabId: string, direction: -1 | 1): void {
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

function categoryName(section: HomeSection): string {
  return selectedCategory(section.sourceId, section.categoryId)?.title ?? section.categoryId;
}

function categoryChoices(section: HomeSection): DiscoveryCategory[] {
  const choices = selectableCategories(section.sourceId);
  if (section.categoryId && !choices.some((category) => category.categoryId === section.categoryId)) {
    return [{ categoryId: section.categoryId, title: `${section.categoryId}（已保存）` }, ...choices];
  }
  return choices;
}

function changeSectionSource(section: HomeSection, sourceId: string): void {
  section.sourceId = sourceId;
  section.categoryId = "";
  void loadHomeEditorCategories(sourceId);
}

function changeSectionCategory(section: HomeSection, categoryId: string): void {
  section.categoryId = categoryId;
}

function updateSectionStyle(section: HomeSection, event: Event): void {
  const value = Number((event.target as HTMLSelectElement).value);
  if (value >= 0 && value <= 3) section.style = value as HomeSection["style"];
}

function addSection(): void {
  const tab = activeTab.value;
  const category = selectedCategory(newSectionSourceId.value, newSectionCategoryId.value);
  if (!tab || !category?.categoryId || tab.sections.length >= 64) return;
  const name = newSectionTitle.value.trim() || category.title;
  tab.sections.push({
    id: nextId("section"),
    title: name,
    sourceId: newSectionSourceId.value,
    categoryId: category.categoryId,
    style: 0,
    sortOrder: tab.sections.length,
    coverVideo: false,
  });
  newSectionCategoryId.value = "";
  newSectionTitle.value = "";
  addingSection.value = false;
}

function removeSection(sectionId: string): void {
  if (!activeTab.value) return;
  activeTab.value.sections = activeTab.value.sections.filter((section) => section.id !== sectionId);
  activeTab.value.sections.forEach((section, sortOrder) => { section.sortOrder = sortOrder; });
}

function moveSection(sectionId: string, direction: -1 | 1): void {
  if (!activeTab.value) return;
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
  if (!event.dataTransfer) { event.preventDefault(); return; }
  draggingSectionId.value = sectionId;
  dropSectionId.value = null;
  event.dataTransfer.effectAllowed = "move";
  event.dataTransfer.setData("text/plain", sectionId);
}
function trackSectionDrop(event: DragEvent, targetId: string): void {
  if (!draggingSectionId.value) return;
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
  if (!movedId || !activeTab.value || movedId === targetId) { finishSectionDrag(); return; }
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

</script>

<template>
  <section class="home-config-editor my-settings-subpage">
    <SubpageHeader class="home-config-editor__topbar col-span-full row-start-1" title="发现页配置" subtitle="修改会自动保存" back-label="返回" @back="returnFromHomeConfig" ></SubpageHeader>
    <div class="home-config-editor__body">
      <div class="home-config-editor__tabs-row">
        <div class="home-config-editor__tabs">
          <KButton rounded :tonal="activeTab?.id === tab.id" :clear="!(activeTab?.id === tab.id)" inline class="gap-2"
            v-for="tab in orderedTabs"
            :key="tab.id"
            type="button"
            :class="{ active: activeTab?.id === tab.id }"
            @click="activeTabId = tab.id"
          >{{ tab.title }}</KButton>
          <KButton rounded inline clear type="button" class="home-config-editor__tab-add gap-2" :disabled="draft.tabs.length >= 32"
            @click="addTab"><PrototypeIcon name="plus"/><span>添加</span></KButton>
        </div>
      </div>

      <details v-if="activeTab" class="home-config-editor__tab-details">
        <summary><PrototypeIcon name="tune"/> 编辑「{{ activeTab.title }}」标签 <PrototypeIcon name="right"/></summary>
        <section class="home-config-editor__tab-settings">
        <label class="home-config-editor__field home-config-editor__tab-name">
          <span>标签页名称</span>
          <input v-model="activeTab.title" maxlength="128" />
        </label>
        <div class="home-config-editor__tab-actions">
          <KButton rounded inline tonal type="button" class="home-config-editor__button home-config-editor__button--secondary gap-2" :disabled="orderedTabs.findIndex((tab) => tab.id === activeTab?.id) === 0" @click="moveTab(activeTab.id, -1)">上移</KButton>
          <KButton rounded inline tonal type="button" class="home-config-editor__button home-config-editor__button--secondary gap-2" :disabled="orderedTabs.findIndex((tab) => tab.id === activeTab?.id) === orderedTabs.length - 1" @click="moveTab(activeTab.id, 1)">下移</KButton>
          <KButton rounded inline clear type="button" class="home-config-editor__button home-config-editor__button--danger k-color-brand-danger gap-2 " :disabled="draft.tabs.length <= 1" @click="deleteActiveTab">删除标签页</KButton>
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
            <span class="home-config-editor__order-grip" draggable="true"
              title="拖动排序；移动端可使用右侧上下按钮"
              @dragstart="beginSectionDrag($event, section.id)" @dragend="finishSectionDrag"><i></i><i></i><i></i></span>
            <label class="home-config-editor__field home-config-editor__section-title">
              <input v-model="section.title" maxlength="128"
                />
              <small :title="sourceName(section.sourceId) + ' · ' + categoryName(section)">{{ sourceName(section.sourceId) }} · {{ categoryName(section) }}</small>
            </label>
            <label class="home-config-editor__compact-style">
              <select :value="section.style"
@change="updateSectionStyle(section, $event)">
                <option :value="0">横向封面</option><option :value="1">排行列表</option><option :value="2">无限网格</option><option :value="3">四列卡片</option>
              </select>
            </label>
            <KButton rounded inline clear type="button" class="home-config-editor__expand gap-2"
              @click="expandedSectionId = expandedSectionId === section.id ? null : section.id">
              <PrototypeIcon name="more"/>
            </KButton>
          </div>

          <div v-if="expandedSectionId === section.id" class="home-config-editor__section-fields">
            <label class="home-config-editor__field">
              <span>书籍来源</span>
              <select :value="section.sourceId" @change="changeSectionSource(section, ($event.target as HTMLSelectElement).value)">
                <option v-for="source in enabledNovelSources" :key="source.id" :value="source.id">{{ source.name }}</option>
              </select>
            </label>
            <label class="home-config-editor__field">
              <span>分类</span>
              <select :value="section.categoryId" :disabled="!section.sourceId" @focus="loadHomeEditorCategories(section.sourceId)" @change="changeSectionCategory(section, ($event.target as HTMLSelectElement).value)">
                <option value="" disabled>{{ categoryPlaceholder(section.sourceId) }}</option>
                <option v-for="category in categoryChoices(section)" :key="category.categoryId" :value="category.categoryId">{{ category.title }}</option>
              </select>
              <small v-if="categoryErrors[section.sourceId]" class="home-config-editor__category-error">{{ categoryErrors[section.sourceId] }} <KButton rounded inline clear class="gap-2" type="button" @click="loadHomeEditorCategories(section.sourceId)">重新读取</KButton></small>
              <small v-else-if="!categoriesLoaded(section.sourceId)">聚焦分类框可读取此来源的其他分类。</small>
            </label>
            <label class="home-config-editor__checkbox-field">
              <KToggle component="span" :checked="!!section.coverVideo" @change="section.coverVideo = ($event.target as HTMLInputElement).checked"  />
              <span>展示封面动画</span>
            </label>
            <div class="home-config-editor__section-actions">
              <KButton rounded inline clear type="button" class="home-config-editor__icon-button gap-2 size-11 p-0"
                :disabled="index === 0" @click="moveSection(section.id, -1)">
                <PrototypeIcon class="section-arrow-up" name="right"/>
              </KButton>
              <KButton rounded inline clear type="button" class="home-config-editor__icon-button gap-2 size-11 p-0"
                :disabled="index === orderedSections.length - 1" @click="moveSection(section.id, 1)">
                <PrototypeIcon class="section-arrow-down" name="right"/>
              </KButton>
              <KButton rounded inline clear type="button" class="home-config-editor__button home-config-editor__button--danger k-color-brand-danger gap-2 "
                @click="removeSection(section.id)">
                移除栏目
              </KButton>
            </div>
          </div>
        </article>

        <section class="home-config-editor__add-section">
          <KButton rounded inline clear type="button" class="home-config-editor__add-toggle gap-2"
            :disabled="!enabledNovelSources.length"
            @click="addingSection = !addingSection">{{ addingSection ? '收起添加栏目' : '+ 添加栏目' }}</KButton>
          <div v-if="addingSection" class="home-config-editor__add-grid">
            <label class="home-config-editor__field">
              <span>书籍来源</span>
              <select v-model="newSectionSourceId" :disabled="!enabledNovelSources.length">
                <option value="" disabled>选择书籍来源</option>
                <option v-for="source in enabledNovelSources" :key="source.id" :value="source.id">{{ source.name }}</option>
              </select>
            </label>
            <label class="home-config-editor__field">
              <span>分类</span>
              <select v-model="newSectionCategoryId" :disabled="!newSectionSourceId">
                <option value="" disabled>{{ categoryPlaceholder(newSectionSourceId) }}</option>
                <option v-for="category in categoriesForNewSection" :key="category.categoryId" :value="category.categoryId">{{ category.title }}</option>
              </select>
            </label>
            <label class="home-config-editor__field">
              <span>栏目名称（可选）</span>
              <input v-model="newSectionTitle" maxlength="128" placeholder="默认使用分类名称" />
            </label>
            <KButton rounded inline type="button" class="home-config-editor__button home-config-editor__button--primary gap-2" :disabled="!activeTab || activeTab.sections.length >= 64 || !selectedCategory(newSectionSourceId, newSectionCategoryId)" @click="addSection">添加栏目</KButton>
          </div>
          <p v-if="categoryErrors[newSectionSourceId]" class="home-config-editor__category-error">{{ categoryErrors[newSectionSourceId] }} <KButton rounded inline clear class="gap-2" type="button" :disabled="!newSectionSourceId" @click="loadHomeEditorCategories(newSectionSourceId)">重新读取分类</KButton></p>
          <p v-if="!enabledNovelSources.length" class="home-config-editor__hint">没有已启用的书籍来源可供选择。</p>
        </section>
      </section>

      <div v-if="validationErrors.length" class="home-config-editor__validation">
        <strong>保存前请处理以下问题</strong>
        <ul><li v-for="error in validationErrors" :key="error">{{ error }}</li></ul>
      </div>
    </div>

  </section>
</template>

<style scoped>
@layer components {
.home-config-editor{
  --hc-accent:var(--app-accent);
  --hc-line:var(--app-line);
  --hc-muted:var(--app-subtle);
  box-sizing:border-box;
  width:100%;
  min-width:0;
  min-height:100%;
  background:var(--app-background);
  color:var(--app-text);
}
.home-config-editor__button{
  display:inline-flex;align-items:center;justify-content:center;gap:6px;
  min-height:34px;padding:0 12px;
  border:1px solid transparent;border-radius:9px;
  font-size:var(--ui-font-body);font-weight:650;cursor:pointer;
}
.home-config-editor__button:disabled,.home-config-editor__icon-button:disabled{opacity:.45;cursor:default}
.home-config-editor__button--primary{background:var(--hc-accent);color:var(--app-on-accent)}
.home-config-editor__button--secondary{border-color:var(--hc-line);background:var(--app-surface);color:var(--app-muted)}
.home-config-editor__button--danger{background:var(--app-danger-soft);color:var(--app-danger)}
.home-config-editor__body{
  box-sizing:border-box;width:100%;
  margin:0 auto;padding:var(--ui-space-6) var(--ui-page-gutter) var(--ui-space-8);max-width:var(--app-content-width);
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
  background:var(--app-panel-soft);color:var(--app-muted);font-size:var(--ui-font-body);
  font-weight:600;white-space:nowrap;cursor:pointer;
}
.home-config-editor__tabs button.active{color:var(--app-text);font-weight:750}
.home-config-editor__tabs .home-config-editor__tab-add{
  gap:4px;border:1px dashed var(--app-accent-soft);background:var(--app-panel-soft);color:var(--hc-accent);
}
.home-config-editor__tab-add :deep(svg){width:13px;height:13px}
.home-config-editor__tab-details{
  margin:0 0 13px;border-top:0;border-bottom:1px solid var(--hc-line);
}
.home-config-editor__tab-details>summary{
  display:flex;align-items:center;gap:8px;
  min-height:34px;padding:0 4px;list-style:none;
  color:var(--app-muted);font-size:var(--ui-font-body);cursor:pointer;
}
.home-config-editor__tab-details>summary::-webkit-details-marker{display:none}
.home-config-editor__tab-details>summary>.prototype-icon{width:15px;height:15px;color:var(--app-accent-ink)}
.home-config-editor__tab-details>summary>.prototype-icon:last-child{margin-left:auto;color:var(--app-subtle)}
.home-config-editor__tab-details[open]>summary>.prototype-icon:last-child{transform:rotate(90deg)}
.home-config-editor__tab-settings{
  display:flex;align-items:end;gap:10px;
  padding:9px 4px 14px;min-width:0;
}
.home-config-editor__tab-name{flex:1}
.home-config-editor__tab-actions{display:flex;align-items:center;gap:6px}
.home-config-editor__field{display:flex;flex-direction:column;gap:4px;min-width:0;color:var(--app-muted);font-size:var(--ui-font-caption)}
.home-config-editor__field>span{font-weight:600}
.home-config-editor__field input,.home-config-editor__field select{
  box-sizing:border-box;width:100%;min-width:0;height:36px;min-height:36px;
  padding:0 9px;border:1px solid var(--hc-line);border-radius:8px;
  background:var(--app-surface);color:var(--app-text);font:inherit;font-size:var(--ui-font-body);
}
.home-config-editor__field small{color:var(--hc-muted);font-size:var(--ui-font-caption);line-height:1.45}
.home-config-editor__category-error{color:var(--app-danger)!important;font-size:var(--ui-font-caption)}
.home-config-editor__category-error button{
  margin-left:4px;border:0;background:transparent;color:var(--hc-accent);
  font-size:var(--ui-font-body);text-decoration:underline;cursor:pointer;
}
.home-config-editor__section-heading{
  display:flex;align-items:center;justify-content:space-between;
  gap:10px;margin:3px 0 8px;color:var(--hc-muted);font-size:var(--ui-font-caption);
}
.home-config-editor__section-heading>span{white-space:nowrap}
.home-config-editor__empty{
  display:grid;place-items:center;min-height:90px;
  padding:16px 0;color:var(--app-subtle);font-size:var(--ui-font-caption);text-align:center;
}
.home-config-editor__sections{min-width:0}
.home-config-editor__section-card{
  box-sizing:border-box;min-height:58px;
  margin:0 0 8px;padding:10px;
  border:1px solid var(--hc-line);border-radius:11px;background:var(--app-surface);
}
.home-config-editor__section-card.is-drop-target{border-color:var(--hc-accent);box-shadow:0 0 0 2px var(--app-accent-soft)}
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
  width:3px;height:13px;border-left:3px dotted var(--app-line);
}
.home-config-editor__section-title{display:flex;flex-direction:column;gap:2px;min-width:0}
.home-config-editor__section-title input{
  box-sizing:border-box;width:100%;height:20px;min-height:20px;
  padding:0;border:0;border-radius:0;outline-offset:2px;
  background:transparent;color:var(--app-text);font-size:var(--ui-font-body);font-weight:700;
}
.home-config-editor__section-title small{
  min-width:0;overflow:hidden;white-space:nowrap;text-overflow:ellipsis;
  color:var(--app-subtle);font-size:var(--ui-font-caption);font-weight:400;
}
.home-config-editor__compact-style select{
  box-sizing:border-box;width:100%;height:31px;min-height:31px;
  padding:0 7px;border:1px solid var(--hc-line);border-radius:8px;
  background:var(--app-surface);color:var(--app-text);font-size:var(--ui-font-body);
}
.home-config-editor__expand{
  display:grid;place-items:center;width:36px;height:36px;
  border:0;border-radius:9px;background:transparent;color:var(--app-muted);cursor:pointer;
}
.home-config-editor__expand :deep(svg){width:18px;height:18px}
.home-config-editor__section-fields{
  display:grid;grid-template-columns:1fr 1fr auto;
  align-items:end;gap:10px;margin-top:9px;padding:13px 5px 3px;
  border-top:1px solid var(--hc-line);
}
.home-config-editor__checkbox-field{
  display:flex;align-items:center;gap:6px;min-height:36px;
  color:var(--app-muted);font-size:var(--ui-font-caption);white-space:nowrap;
}
.home-config-editor__checkbox-field input{width:16px;height:16px;accent-color:var(--hc-accent)}
.home-config-editor__section-actions{
  display:flex;justify-content:flex-end;align-items:center;
  grid-column:1/-1;gap:5px;
}
.home-config-editor__icon-button{
  display:grid;place-items:center;width:32px;height:32px;padding:0;
  border:1px solid var(--hc-line);border-radius:8px;
  background:var(--app-surface);color:var(--app-accent-ink);cursor:pointer;
}
.home-config-editor__icon-button :deep(svg){width:16px;height:16px}
.home-config-editor__icon-button .section-arrow-up{transform:rotate(-90deg)}
.home-config-editor__icon-button .section-arrow-down{transform:rotate(90deg)}
.home-config-editor__add-section{margin-top:10px}
.home-config-editor__add-toggle{
  box-sizing:border-box;width:100%;min-height:41px;
  border:1px dashed var(--app-accent-soft);border-radius:11px;
  background:var(--app-panel-soft);color:var(--hc-accent);
  text-align:center;font-size:var(--ui-font-caption);font-weight:700;cursor:pointer;
}
.home-config-editor__add-toggle:disabled{opacity:.5;cursor:default}
.home-config-editor__add-grid{
  display:grid;grid-template-columns:1fr 1fr 1.1fr auto;
  align-items:end;gap:10px;margin-top:12px;padding:11px;
  border:1px solid var(--hc-line);border-radius:10px;background:var(--app-surface);
}
.home-config-editor__hint{margin:9px 0;color:var(--hc-muted);font-size:var(--ui-font-caption)}
.home-config-editor__validation{
  margin-top:14px;padding:11px 12px;border:1px solid var(--app-danger-line);
  border-radius:9px;background:var(--app-danger-soft);color:var(--app-danger);
  font-size:var(--ui-font-caption);line-height:1.6;
}
.home-config-editor__validation ul{margin:7px 0 0;padding-left:17px}
.home-config-editor__discard-mask{
  position:fixed;inset:0;z-index:120;
  display:grid;place-items:center;padding:16px;background:var(--app-scrim);
}
.home-config-editor__discard-dialog{
  box-sizing:border-box;width:min(430px,100%);padding:22px;
  border:1px solid var(--hc-line);border-radius:14px;
  background:var(--app-surface);box-shadow:0 16px 44px #11142822;
}
.home-config-editor__discard-dialog h3{margin:0;color:var(--app-text);font-size:var(--ui-font-title)}
.home-config-editor__discard-dialog p{margin:10px 0;color:var(--app-muted);font-size:var(--ui-font-caption);line-height:1.65}
.home-config-editor__discard-dialog footer{display:flex;justify-content:flex-end;gap:8px;margin-top:17px}
@media(max-width:860px){
  .home-config-editor__body{padding:var(--ui-space-6) var(--ui-page-gutter) var(--ui-space-8)}
  .home-config-editor__section-title-row{grid-template-columns:18px minmax(0,1fr) 100px 32px;gap:8px}
  .home-config-editor__expand{width:32px}
  .home-config-editor__section-fields{grid-template-columns:repeat(2,minmax(0,1fr))}
  .home-config-editor__checkbox-field{grid-column:1/-1}
}
@media(max-width:560px){
  .home-config-editor__body{width:100%}
  .home-config-editor__section-title-row{grid-template-columns:18px minmax(0,1fr) 32px}
  .home-config-editor__compact-style{grid-column:2;grid-row:2}
  .home-config-editor__expand{grid-column:3;grid-row:1/3}
  .home-config-editor__section-fields{grid-template-columns:minmax(0,1fr)}
  .home-config-editor__checkbox-field{grid-column:1}
  .home-config-editor__tab-settings{flex-direction:column;align-items:stretch}
  .home-config-editor__tab-actions{flex-wrap:wrap}
  .home-config-editor__add-grid{grid-template-columns:minmax(0,1fr)}
}
}
</style>

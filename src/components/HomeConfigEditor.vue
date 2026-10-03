<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type { DiscoveryCategory, HomeConfigDocument, HomeSection, HomeTab, SourceMetadata } from "../api/app";

const props = withDefaults(
  defineProps<{
    homeConfig: HomeConfigDocument;
    sources: SourceMetadata[];
    categoriesBySource: Record<string, DiscoveryCategory[]>;
    loadingSourceIds?: string[];
    categoryErrors?: Record<string, string>;
    busy?: boolean;
  }>(),
  { loadingSourceIds: () => [], categoryErrors: () => ({}), busy: false },
);

const emit = defineEmits<{
  save: [config: HomeConfigDocument];
  cancel: [];
  loadCategories: [sourceId: string];
}>();

const draft = ref<HomeConfigDocument>({ schemaVersion: 1, tabs: [] });
const activeTabId = ref("");
const initialSnapshot = ref("");
const newSectionSourceId = ref("");
const newSectionCategoryId = ref("");
const newSectionTitle = ref("");

const enabledNovelSources = computed(() => props.sources
  .filter((source) => source.enabled && source.isRss !== true)
  .slice()
  .sort((left, right) => left.name.localeCompare(right.name, "zh-CN")));
const orderedTabs = computed(() => [...draft.value.tabs].sort((left, right) => left.sortOrder - right.sortOrder));
const activeTab = computed(() => orderedTabs.value.find((tab) => tab.id === activeTabId.value) ?? orderedTabs.value[0]);
const orderedSections = computed(() => [...(activeTab.value?.sections ?? [])].sort((left, right) => left.sortOrder - right.sortOrder));
const categoriesForNewSection = computed(() => selectableCategories(newSectionSourceId.value));
const isDirty = computed(() => JSON.stringify(normalizeDraft(draft.value)) !== initialSnapshot.value);

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
      const source = props.sources.find((entry) => entry.id === section.sourceId);
      if (!source || !source.enabled || source.isRss === true) errors.push(`栏目“${sectionTitle || section.categoryName}”需要选择已启用的书籍来源。`);
      if (!section.categoryId.trim()) errors.push(`栏目“${sectionTitle || "未命名"}”需要选择一个分类。`);
      if (section.style < 0 || section.style > 3) errors.push(`栏目“${sectionTitle || "未命名"}”的展示样式无效。`);
    }
  }
  return [...new Set(errors)];
});

const canSave = computed(() => isDirty.value && validationErrors.value.length === 0 && !props.busy);

watch(
  () => props.homeConfig,
  (value) => {
    const copy = cloneDocument(value);
    draft.value = normalizeDraft(copy);
    initialSnapshot.value = JSON.stringify(draft.value);
    if (!draft.value.tabs.some((tab) => tab.id === activeTabId.value)) activeTabId.value = draft.value.tabs[0]?.id ?? "";
  },
  { immediate: true, deep: true },
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
  if (sourceId) emit("loadCategories", sourceId);
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
  if (props.busy || draft.value.tabs.length >= 32) return;
  const tab: HomeTab = { id: nextId("tab"), title: nextUniqueTabTitle(), sortOrder: draft.value.tabs.length, sections: [] };
  draft.value.tabs.push(tab);
  activeTabId.value = tab.id;
}

function deleteActiveTab(): void {
  if (props.busy || !activeTab.value || draft.value.tabs.length <= 1) return;
  const index = orderedTabs.value.findIndex((tab) => tab.id === activeTab.value?.id);
  draft.value.tabs = orderedTabs.value.filter((tab) => tab.id !== activeTab.value?.id);
  draft.value.tabs.forEach((tab, order) => { tab.sortOrder = order; });
  activeTabId.value = draft.value.tabs[Math.max(0, index - 1)]?.id ?? draft.value.tabs[0]?.id ?? "";
}

function moveTab(tabId: string, direction: -1 | 1): void {
  if (props.busy) return;
  const tabs = orderedTabs.value;
  const index = tabs.findIndex((tab) => tab.id === tabId);
  const target = index + direction;
  if (index < 0 || target < 0 || target >= tabs.length) return;
  [tabs[index], tabs[target]] = [tabs[target], tabs[index]];
  draft.value.tabs = tabs.map((tab, sortOrder) => ({ ...tab, sortOrder }));
}

function selectableCategories(sourceId: string): DiscoveryCategory[] {
  return (props.categoriesBySource[sourceId] ?? []).filter((category) => Boolean(category.categoryId));
}

function categoryPlaceholder(sourceId: string): string {
  if (!sourceId) return "先选择书籍来源";
  if (props.categoryErrors[sourceId]) return "读取失败，请重试";
  if (props.loadingSourceIds.includes(sourceId)) return "正在读取分类…";
  if (!categoriesLoaded(sourceId)) return "等待读取分类";
  return selectableCategories(sourceId).length ? "选择分类" : "没有可用分类";
}

function categoriesLoaded(sourceId: string): boolean {
  return Object.prototype.hasOwnProperty.call(props.categoriesBySource, sourceId);
}

function selectedCategory(sourceId: string, categoryId: string): DiscoveryCategory | undefined {
  return selectableCategories(sourceId).find((category) => category.categoryId === categoryId);
}

function sourceName(sourceId: string): string {
  return props.sources.find((source) => source.id === sourceId)?.name ?? "已保存来源";
}

function categoryChoices(section: HomeSection): DiscoveryCategory[] {
  const choices = selectableCategories(section.sourceId);
  if (section.categoryId && !choices.some((category) => category.categoryId === section.categoryId)) {
    return [{ categoryId: section.categoryId, title: `${section.categoryName}（已保存）` }, ...choices];
  }
  return choices;
}

function changeSectionSource(section: HomeSection, sourceId: string): void {
  if (props.busy) return;
  section.sourceId = sourceId;
  section.sourceName = sourceName(sourceId);
  section.categoryId = "";
  section.categoryName = "";
  emit("loadCategories", sourceId);
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
  if (props.busy || !tab || !category?.categoryId || tab.sections.length >= 64) return;
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
}

function removeSection(sectionId: string): void {
  if (props.busy || !activeTab.value) return;
  activeTab.value.sections = activeTab.value.sections.filter((section) => section.id !== sectionId);
  activeTab.value.sections.forEach((section, sortOrder) => { section.sortOrder = sortOrder; });
}

function moveSection(sectionId: string, direction: -1 | 1): void {
  if (props.busy || !activeTab.value) return;
  const sections = orderedSections.value;
  const index = sections.findIndex((section) => section.id === sectionId);
  const target = index + direction;
  if (index < 0 || target < 0 || target >= sections.length) return;
  [sections[index], sections[target]] = [sections[target], sections[index]];
  activeTab.value.sections = sections.map((section, sortOrder) => ({ ...section, sortOrder }));
}

function submit(): void {
  if (!canSave.value) return;
  emit("save", normalizeDraft(cloneDocument(draft.value)));
}
</script>

<template>
  <section class="home-config-editor" role="dialog" aria-modal="true" aria-labelledby="home-config-title" data-testid="home-config-editor">
    <header class="home-config-editor__header">
      <div>
        <p class="home-config-editor__eyebrow">主页设置</p>
        <h2 id="home-config-title">管理主页栏目</h2>
        <p class="home-config-editor__intro">调整标签页、栏目顺序和展示样式。分类由应用读取，保存时会再次核验。</p>
      </div>
      <button type="button" class="home-config-editor__icon-button" aria-label="关闭主页设置" data-testid="home-config-close" @click="emit('cancel')">×</button>
    </header>

    <div class="home-config-editor__body">
      <div class="home-config-editor__tabs-row">
        <div class="home-config-editor__tabs" role="tablist" aria-label="主页标签页">
          <button
            v-for="tab in orderedTabs"
            :key="tab.id"
            type="button"
            role="tab"
            :aria-selected="activeTab?.id === tab.id"
            :data-testid="`home-config-tab-${tab.id}`"
            @click="activeTabId = tab.id"
          >{{ tab.title }}</button>
        </div>
        <button type="button" class="home-config-editor__button home-config-editor__button--secondary" data-testid="home-config-add-tab" :disabled="busy || draft.tabs.length >= 32" @click="addTab">添加标签页</button>
      </div>

      <section v-if="activeTab" class="home-config-editor__tab-settings" :aria-label="`${activeTab.title}设置`">
        <label class="home-config-editor__field home-config-editor__tab-name">
          <span>标签页名称</span>
          <input v-model="activeTab.title" maxlength="128" :disabled="busy" :data-testid="`home-config-tab-name-${activeTab.id}`" />
        </label>
        <div class="home-config-editor__tab-actions">
          <button type="button" class="home-config-editor__button home-config-editor__button--secondary" :data-testid="`home-config-tab-up-${activeTab.id}`" :disabled="busy || orderedTabs.findIndex((tab) => tab.id === activeTab?.id) === 0" aria-label="标签页上移" @click="moveTab(activeTab.id, -1)">上移</button>
          <button type="button" class="home-config-editor__button home-config-editor__button--secondary" :data-testid="`home-config-tab-down-${activeTab.id}`" :disabled="busy || orderedTabs.findIndex((tab) => tab.id === activeTab?.id) === orderedTabs.length - 1" aria-label="标签页下移" @click="moveTab(activeTab.id, 1)">下移</button>
          <button type="button" class="home-config-editor__button home-config-editor__button--danger" data-testid="home-config-delete-tab" :disabled="busy || draft.tabs.length <= 1" @click="deleteActiveTab">删除标签页</button>
        </div>
      </section>

      <section class="home-config-editor__sections" aria-label="主页栏目">
        <div class="home-config-editor__section-heading">
          <div><h3>栏目</h3><p>一个栏目对应一个已加载的书籍分类。</p></div>
          <span>{{ activeTab?.sections.length ?? 0 }} / 64</span>
        </div>

        <div v-if="!orderedSections.length" class="home-config-editor__empty" data-testid="home-config-empty-sections">这个标签页还没有栏目。</div>

        <article v-for="(section, index) in orderedSections" :key="section.id" class="home-config-editor__section-card" :data-testid="`home-config-section-${section.id}`">
          <div class="home-config-editor__section-title-row">
            <label class="home-config-editor__field home-config-editor__section-title">
              <span>栏目名称</span>
              <input v-model="section.title" maxlength="128" :disabled="busy" :data-testid="`home-config-section-title-${section.id}`" />
            </label>
            <div class="home-config-editor__section-actions">
              <button type="button" class="home-config-editor__icon-button" :data-testid="`home-config-section-up-${section.id}`" :disabled="busy || index === 0" :aria-label="`${section.title}上移`" @click="moveSection(section.id, -1)">↑</button>
              <button type="button" class="home-config-editor__icon-button" :data-testid="`home-config-section-down-${section.id}`" :disabled="busy || index === orderedSections.length - 1" :aria-label="`${section.title}下移`" @click="moveSection(section.id, 1)">↓</button>
              <button type="button" class="home-config-editor__icon-button home-config-editor__icon-button--danger" :data-testid="`home-config-section-remove-${section.id}`" :disabled="busy" :aria-label="`移除${section.title}`" @click="removeSection(section.id)">×</button>
            </div>
          </div>

          <div class="home-config-editor__section-fields">
            <label class="home-config-editor__field">
              <span>书籍来源</span>
              <select :value="section.sourceId" :data-testid="`home-config-section-source-${section.id}`" :disabled="busy" @change="changeSectionSource(section, ($event.target as HTMLSelectElement).value)">
                <option v-for="source in enabledNovelSources" :key="source.id" :value="source.id">{{ source.name }}</option>
              </select>
            </label>
            <label class="home-config-editor__field">
              <span>分类</span>
              <select :value="section.categoryId" :data-testid="`home-config-section-category-${section.id}`" :disabled="busy || !section.sourceId" @focus="emit('loadCategories', section.sourceId)" @change="changeSectionCategory(section, ($event.target as HTMLSelectElement).value)">
                <option value="" disabled>{{ categoryPlaceholder(section.sourceId) }}</option>
                <option v-for="category in categoryChoices(section)" :key="category.categoryId" :value="category.categoryId">{{ category.title }}</option>
              </select>
              <small v-if="categoryErrors[section.sourceId]" class="home-config-editor__category-error" role="alert">{{ categoryErrors[section.sourceId] }} <button type="button" :disabled="busy" @click="emit('loadCategories', section.sourceId)">重新读取</button></small>
              <small v-else-if="!categoriesLoaded(section.sourceId)">聚焦分类框可读取此来源的其他分类。</small>
            </label>
            <label class="home-config-editor__field">
              <span>展示样式</span>
              <select :value="section.style" :data-testid="`home-config-section-style-${section.id}`" :disabled="busy" @change="updateSectionStyle(section, $event)">
                <option :value="0">横向封面</option>
                <option :value="1">排行列表</option>
                <option :value="2">无限网格（每页最多一个）</option>
                <option :value="3">四列卡片</option>
              </select>
            </label>
            <label class="home-config-editor__checkbox-field">
              <input v-model="section.coverVideo" type="checkbox" :disabled="busy" />
              <span>展示封面动画</span>
            </label>
          </div>
        </article>

        <section class="home-config-editor__add-section" aria-label="添加栏目">
          <h4>添加栏目</h4>
          <div class="home-config-editor__add-grid">
            <label class="home-config-editor__field">
              <span>书籍来源</span>
              <select v-model="newSectionSourceId" :disabled="busy || !enabledNovelSources.length" data-testid="home-config-new-source">
                <option value="" disabled>选择书籍来源</option>
                <option v-for="source in enabledNovelSources" :key="source.id" :value="source.id">{{ source.name }}</option>
              </select>
            </label>
            <label class="home-config-editor__field">
              <span>分类</span>
              <select v-model="newSectionCategoryId" :disabled="busy || !newSectionSourceId" data-testid="home-config-new-category">
                <option value="" disabled>{{ categoryPlaceholder(newSectionSourceId) }}</option>
                <option v-for="category in categoriesForNewSection" :key="category.categoryId" :value="category.categoryId">{{ category.title }}</option>
              </select>
            </label>
            <label class="home-config-editor__field">
              <span>栏目名称（可选）</span>
              <input v-model="newSectionTitle" maxlength="128" :disabled="busy" placeholder="默认使用分类名称" data-testid="home-config-new-title" />
            </label>
            <button type="button" class="home-config-editor__button home-config-editor__button--primary" data-testid="home-config-add-section" :disabled="busy || !activeTab || activeTab.sections.length >= 64 || !selectedCategory(newSectionSourceId, newSectionCategoryId)" @click="addSection">添加栏目</button>
          </div>
          <p v-if="categoryErrors[newSectionSourceId]" class="home-config-editor__category-error" role="alert">{{ categoryErrors[newSectionSourceId] }} <button type="button" :disabled="busy || !newSectionSourceId" @click="emit('loadCategories', newSectionSourceId)">重新读取分类</button></p>
          <p v-if="!enabledNovelSources.length" class="home-config-editor__hint">没有已启用的书籍来源可供选择。</p>
        </section>
      </section>

      <div v-if="validationErrors.length" class="home-config-editor__validation" role="alert" data-testid="home-config-validation">
        <strong>保存前请处理以下问题</strong>
        <ul><li v-for="error in validationErrors" :key="error">{{ error }}</li></ul>
      </div>
    </div>

    <footer class="home-config-editor__footer">
      <button type="button" class="home-config-editor__button home-config-editor__button--secondary" :disabled="busy" @click="emit('cancel')">取消</button>
      <button type="button" class="home-config-editor__button home-config-editor__button--primary" data-testid="home-config-save" :disabled="!canSave" @click="submit">{{ busy ? '正在保存…' : '保存设置' }}</button>
    </footer>
  </section>
</template>

<style scoped>
.home-config-editor { width:min(820px,100%); max-height:min(900px,94dvh); display:flex; flex-direction:column; overflow:hidden; border:1px solid #dfe7df; border-radius:16px; color:#34463a; background:#fbfcfa; box-shadow:0 18px 60px #17251b33; }
.home-config-editor__header { min-height:96px; display:flex; align-items:flex-start; justify-content:space-between; gap:16px; padding:18px 22px; border-bottom:1px solid #e4ebe4; }
.home-config-editor__eyebrow { margin:0 0 4px; color:#6b8871; font-size:13px; }
.home-config-editor__header h2 { margin:0; font-size:21px; }
.home-config-editor__intro { margin:6px 0 0; color:#6d7c70; font-size:14px; line-height:1.5; }
.home-config-editor__body { min-height:0; overflow:auto; padding:16px 20px; }
.home-config-editor__tabs-row { display:flex; align-items:center; gap:10px; }
.home-config-editor__tabs { min-width:0; flex:1; display:flex; gap:6px; overflow:auto; }
.home-config-editor__tabs button { min-height:44px; flex:none; padding:0 14px; border:1px solid #dfe7df; border-radius:9px; color:#526356; background:#fff; font-size:14px; cursor:pointer; }
.home-config-editor__tabs button[aria-selected="true"] { border-color:#9cb9a0; color:#355740; background:#edf5ee; font-weight:700; }
.home-config-editor__button,.home-config-editor__icon-button { min-height:44px; display:inline-flex; align-items:center; justify-content:center; gap:6px; padding:0 13px; border:1px solid transparent; border-radius:8px; font:inherit; font-size:14px; cursor:pointer; }
.home-config-editor__button:disabled,.home-config-editor__icon-button:disabled { opacity:.48; cursor:not-allowed; }
.home-config-editor__button--primary { color:#fff; background:#477553; }
.home-config-editor__button--secondary { border-color:#dce5dc; color:#45594a; background:#fff; }
.home-config-editor__button--danger,.home-config-editor__icon-button--danger { border-color:#efd9d5; color:#92544b; background:#fff7f5; }
.home-config-editor__tab-settings { display:flex; align-items:flex-end; gap:12px; margin:16px 0 23px; padding:14px; border:1px solid #e4ebe4; border-radius:11px; background:#f6f9f5; }
.home-config-editor__field { min-width:0; display:flex; flex-direction:column; gap:6px; color:#536456; font-size:14px; }
.home-config-editor__field > span { font-weight:650; }
.home-config-editor__field input,.home-config-editor__field select { width:100%; min-width:0; min-height:44px; padding:0 10px; border:1px solid #d8e1d8; border-radius:8px; color:#34463a; background:#fff; font:inherit; font-size:14px; }
.home-config-editor__field small { color:#79867c; font-size:12px; line-height:1.4; }
.home-config-editor__category-error { color:#8b514b !important; }
.home-config-editor__category-error button { min-height:34px; margin-left:4px; border:0; color:#557a61; background:transparent; font:inherit; text-decoration:underline; cursor:pointer; }
.home-config-editor__tab-name { flex:1; }
.home-config-editor__tab-actions,.home-config-editor__section-actions { display:flex; align-items:center; gap:6px; }
.home-config-editor__section-heading { display:flex; align-items:center; justify-content:space-between; margin-bottom:10px; }
.home-config-editor__section-heading h3 { margin:0; font-size:17px; }
.home-config-editor__section-heading p,.home-config-editor__hint { margin:3px 0 0; color:#718074; font-size:13px; }
.home-config-editor__section-heading > span { color:#718074; font-size:13px; }
.home-config-editor__empty { padding:22px; border:1px dashed #d7e1d7; border-radius:10px; color:#718074; background:#f8faf7; font-size:14px; text-align:center; }
.home-config-editor__section-card { margin:9px 0; padding:13px; border:1px solid #e1e9e1; border-radius:11px; background:#fff; }
.home-config-editor__section-title-row { display:flex; align-items:flex-end; gap:12px; }
.home-config-editor__section-title { flex:1; }
.home-config-editor__icon-button { width:44px; padding:0; border-color:#e1e8e1; color:#536456; background:#fff; font-size:18px; }
.home-config-editor__section-fields { display:grid; grid-template-columns:1fr 1fr 1fr auto; align-items:end; gap:10px; margin-top:12px; }
.home-config-editor__checkbox-field { min-height:44px; display:flex; align-items:center; gap:8px; color:#536456; font-size:13px; white-space:nowrap; }
.home-config-editor__checkbox-field input { width:18px; height:18px; accent-color:#477553; }
.home-config-editor__add-section { margin-top:15px; padding:14px; border:1px solid #e1e9e1; border-radius:11px; background:#f8faf7; }
.home-config-editor__add-section h4 { margin:0 0 10px; font-size:15px; }
.home-config-editor__add-grid { display:grid; grid-template-columns:1fr 1fr 1.2fr auto; align-items:end; gap:10px; }
.home-config-editor__validation { margin-top:15px; padding:12px 14px; border:1px solid #f0d6d2; border-radius:9px; color:#824a43; background:#fff5f3; font-size:14px; line-height:1.5; }
.home-config-editor__validation ul { margin:7px 0 0; padding-left:20px; }
.home-config-editor__footer { min-height:68px; display:flex; justify-content:flex-end; gap:9px; padding:11px 18px; border-top:1px solid #e4ebe4; background:#fff; }
@media(max-width:680px) {
  .home-config-editor { width:100vw; height:100dvh; max-height:none; border:0; border-radius:0; padding-bottom:env(safe-area-inset-bottom); }
  .home-config-editor__header { padding:14px; }
  .home-config-editor__header h2 { font-size:19px; }
  .home-config-editor__body { padding:13px; }
  .home-config-editor__tabs-row { align-items:stretch; flex-direction:column; }
  .home-config-editor__tabs-row > .home-config-editor__button { width:100%; }
  .home-config-editor__tab-settings { align-items:stretch; flex-direction:column; padding:11px; }
  .home-config-editor__tab-actions > .home-config-editor__button { flex:1; }
  .home-config-editor__section-card { padding:10px; }
  .home-config-editor__section-fields,.home-config-editor__add-grid { grid-template-columns:1fr; }
  .home-config-editor__checkbox-field { min-height:44px; }
  .home-config-editor__footer { padding-right:14px; padding-left:14px; }
  .home-config-editor__footer > .home-config-editor__button { flex:1; }
}
</style>

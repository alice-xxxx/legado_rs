<script setup lang="ts">
import { ListInput as KListInput, Popover as KPopover, Checkbox as KCheckbox, Button as KButton } from "konsta/vue";
// 书源页管理本地筛选和选择视图；保存、登录、导入等副作用交给应用层处理。
import { computed, ref, watch } from "vue";
import { useVirtualList } from "../../ui/useVirtualList";
import type { SourceMetadata } from "../../api/types";
import ManagedListRow from "../../ui/ManagedListRow.vue";
import ManagedListToolbar from "../../ui/ManagedListToolbar.vue";
import SubpageHeader from "../../ui/SubpageHeader.vue";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import { useOutsidePointerDismiss } from "../../ui/useOutsidePointerDismiss";

const props = defineProps<{
  sources: SourceMetadata[];
  selectedIds: string[];
  batchBusy: boolean;
  exportBusy: boolean;
  locked: boolean;
  editorOpen?: boolean;
}>();

const emit = defineEmits<{
  back: [];
  "selection-change": [ids: string[]];
  "toggle-source": [id: string];
  "change-enabled": [source: SourceMetadata, enabled: boolean];
  "create-open": [];
  "import-local": [];
  "import-url-open": [];
  "set-group": [group: string | null];
  "set-enabled": [enabled: boolean];
  export: [];
  remove: [ids: string[]];
  edit: [source: SourceMetadata];
  "switch-editor": [source: SourceMetadata];
}>();

const query = ref("");
const groupFilter = ref("");
const statusFilter = ref<"all" | "enabled" | "disabled">("all");
const filterMenuOpen = ref(false);
const actionsMenuOpen = ref(false);
const batchGroup = ref("");
const batchMode = ref(false);
const addMenuOpen = ref(false);
const openSourceMenuId = ref<string | null>(null);
const toolbarBusy = computed(() => props.batchBusy || props.exportBusy);
const writeLocked = computed(() => toolbarBusy.value || props.locked);

const groups = computed(() => [...new Set(props.sources.map((source) => source.group).filter((group): group is string => Boolean(group)))].sort());
const enabledCount = computed(() => props.sources.filter((source) => source.enabled).length);
function sourceKindLabel(source: SourceMetadata): string {
  if (source.isRss) return "RSS";
  if (source.contentType === "image") return "图片";
  if (source.mediaType === "audio") return "音频";
  if (source.mediaType === "video") return "视频";
  return source.searchType === "text" ? "文本" : "其他";
}
const filtersActive = computed(() => Boolean(query.value.trim() || groupFilter.value || statusFilter.value !== "all"));
function clearFilters(): void {
  query.value = "";
  groupFilter.value = "";
  statusFilter.value = "all";
}
const filteredSources = computed(() => {
  const needle = query.value.trim().toLocaleLowerCase("zh-CN");
  return props.sources.filter((source) => {
    if (groupFilter.value && source.group !== groupFilter.value) return false;
    if (statusFilter.value === "enabled" && !source.enabled) return false;
    if (statusFilter.value === "disabled" && source.enabled) return false;
    return !needle || `${source.name} ${source.group ?? ""} ${source.id}`.toLocaleLowerCase("zh-CN").includes(needle);
  });
});
const sourceViewport = ref<HTMLElement | null>(null);
const sourceWindow = useVirtualList(filteredSources, sourceViewport, 64);
const { range: sourceRange, visibleItems: visibleSources } = sourceWindow;
watch([query, groupFilter, statusFilter], () => { void sourceWindow.reset(); });
const selectedSourceSet = computed(() => new Set(props.selectedIds));
const allFilteredSelected = computed(() => filteredSources.value.length > 0
  && filteredSources.value.every((source) => selectedSourceSet.value.has(source.id)));
function toggleFilteredSelection(): void {
  if (props.batchBusy || props.exportBusy) return;
  const next = new Set(props.selectedIds);
  for (const source of filteredSources.value) {
    if (allFilteredSelected.value) next.delete(source.id);
    else next.add(source.id);
  }
  emit("selection-change", [...next]);
}

function removeSelected(): void {
  emit("remove", [...props.selectedIds]);
}

function chooseAddAction(action: "new" | "file" | "url"): void {
  addMenuOpen.value = false;
  if (writeLocked.value || (props.editorOpen && action !== "new")) return;
  if (action === "new") emit("create-open");
  else if (action === "file") emit("import-local");
  else emit("import-url-open");
}

useOutsidePointerDismiss(
  () => addMenuOpen.value || !!openSourceMenuId.value || filterMenuOpen.value || actionsMenuOpen.value,
  (target) => target instanceof Element
    && !!target.closest(".source-add-menu, .source-header-add, .managed-list-toolbar__control, .managed-list-row__menu-wrap"),
  () => {
  addMenuOpen.value = false;
  openSourceMenuId.value = null;
  filterMenuOpen.value = false;
  actionsMenuOpen.value = false;
  },
);
</script>

<template>
  <section class="sources-view source-manager-page" :class="{ 'source-manager-page--editing': editorOpen }">
    <SubpageHeader wide title="内容引擎" back-label="返回我的" @back="emit('back')">
      <template #actions>
        <div class="source-header-add">
          <KButton rounded inline clear type="button" class="source-header-import gap-2"
            :disabled="writeLocked"
            @click="addMenuOpen = !addMenuOpen; openSourceMenuId = null; filterMenuOpen = false; actionsMenuOpen = false"><PrototypeIcon name="plus"/><span>添加</span></KButton>

        </div>
      </template>
    </SubpageHeader>
<KPopover v-if="addMenuOpen" opened :backdrop="false" target=".source-header-add" class="source-add-menu w-60 max-w-[calc(100vw-32px)]">
  <div class="p-2">
<KButton rounded inline clear class="gap-2 w-full justify-start text-current" type="button" @click="chooseAddAction('new')">新建来源</KButton>
<KButton rounded inline clear class="gap-2 w-full justify-start text-current" type="button" :disabled="editorOpen" @click="chooseAddAction('file')">本地 JSON 导入</KButton>
<KButton rounded inline clear class="gap-2 w-full justify-start text-current" type="button" :disabled="editorOpen" @click="chooseAddAction('url')">网络链接导入</KButton>
  </div>
</KPopover>
    <div v-if="locked" class="source-recovery-notice">资源恢复尚未完成。修改来源的操作暂时不可用，请重启应用完成恢复。</div>

    <div class="source-workbench" :class="{ 'is-editing': editorOpen }">
      <aside class="source-master">
        <div class="source-filter-summary">
          <span>{{ enabledCount }} 个启用</span><small>{{ sources.length }} 个配置</small>
        </div>
        <ManagedListToolbar
          v-model:query="query"
          search-placeholder="搜索名称、分组或标识"
          :filter-expanded="filterMenuOpen"
          :actions-expanded="actionsMenuOpen"
          @toggle-filter="filterMenuOpen = !filterMenuOpen; actionsMenuOpen = false; addMenuOpen = false; openSourceMenuId = null"
          @toggle-actions="actionsMenuOpen = !actionsMenuOpen; filterMenuOpen = false; addMenuOpen = false; openSourceMenuId = null"
        >
          <template #filter-menu>
            <KPopover opened :backdrop="false" v-if="filterMenuOpen" class="source-filter-menu w-60 max-w-[calc(100vw-32px)]" target=".managed-list-toolbar [aria-label='筛选列表']"><div class="grid gap-1 p-2">
              <KButton rounded :tonal="statusFilter === 'all'" :clear="!(statusFilter === 'all')" inline class="gap-2 w-full justify-start text-current" type="button" :class="{ active: statusFilter === 'all' }"
                @click="statusFilter = 'all'; filterMenuOpen = false">全部来源</KButton>
              <KButton rounded :tonal="statusFilter === 'enabled'" :clear="!(statusFilter === 'enabled')" inline class="gap-2 w-full justify-start text-current" type="button" :class="{ active: statusFilter === 'enabled' }"
                @click="statusFilter = 'enabled'; filterMenuOpen = false">仅启用</KButton>
              <KButton rounded :tonal="statusFilter === 'disabled'" :clear="!(statusFilter === 'disabled')" inline class="gap-2 w-full justify-start text-current" type="button" :class="{ active: statusFilter === 'disabled' }"
                @click="statusFilter = 'disabled'; filterMenuOpen = false">仅停用</KButton>
              <template v-if="groups.length || groupFilter">
                <div class="source-filter-divider" role="separator"></div>
                <KButton rounded :tonal="!groupFilter" :clear="!(!groupFilter)" inline class="gap-2 w-full justify-start text-current" type="button" :class="{ active: !groupFilter }" :aria-pressed="!groupFilter"
                  @click="groupFilter = ''; filterMenuOpen = false">全部分组</KButton>
                <KButton rounded :tonal="groupFilter === group" :clear="!(groupFilter === group)" inline class="gap-2 w-full justify-start text-current" v-for="group in groups" :key="group" type="button" :class="{ active: groupFilter === group }"
                  :aria-pressed="groupFilter === group" :aria-label="`分组：${group}`"
                  @click="groupFilter = group; filterMenuOpen = false">{{ group }}</KButton>
              </template>
            </div></KPopover>
          </template>
          <template #actions-menu>
            <KPopover opened :backdrop="false" v-if="actionsMenuOpen" class="source-toolbar-actions-menu w-60 max-w-[calc(100vw-32px)]" target=".managed-list-toolbar [aria-label='更多列表操作']"><div class="grid gap-1 p-2">
              <KButton rounded inline clear class="gap-2 w-full justify-start text-current" type="button" :disabled="toolbarBusy || editorOpen"
                @click="batchMode = !batchMode; emit('selection-change', []); actionsMenuOpen = false">
                {{ batchMode ? '退出批量管理' : '批量管理' }}
              </KButton>
            </div></KPopover>
          </template>
        </ManagedListToolbar>
        <div v-if="filteredSources.length" ref="sourceViewport" class="source-master-list" @scroll.passive="filterMenuOpen = false; actionsMenuOpen = false; openSourceMenuId = null">
          <div :ref="sourceWindow.list">
          <div aria-hidden="true" :style="{ height: `${sourceRange.top}px` }"></div>
          <ManagedListRow v-for="source in visibleSources" :key="source.id"
            :name="source.name" :subtitle="`${sourceKindLabel(source)} · ${source.group || '未分组'}`"
            :enabled="source.enabled"
            :busy="writeLocked" :batch-mode="batchMode" :selected="selectedSourceSet.has(source.id)"
            :menu-open="openSourceMenuId === source.id"
            @open="editorOpen ? emit('switch-editor', source) : emit('edit', source)"
            @select="emit('toggle-source', source.id)"
            @toggle-menu="openSourceMenuId = openSourceMenuId === source.id ? null : source.id; addMenuOpen = false; filterMenuOpen = false; actionsMenuOpen = false"
          >
            <template #menu>
                <KButton rounded inline clear class="gap-2" type="button" :disabled="writeLocked"
                  @click="openSourceMenuId = null; emit('change-enabled', source, !source.enabled)">{{ source.enabled ? '停用' : '启用' }}</KButton>
                <KButton rounded inline clear type="button" class="is-danger k-color-brand-danger gap-2 " :disabled="writeLocked"
                  @click="openSourceMenuId = null; emit('remove', [source.id])">删除来源</KButton>
            </template>
          </ManagedListRow>
          <div aria-hidden="true" :style="{ height: `${sourceRange.bottom}px` }"></div>
          </div>
        </div>
        <div v-else class="source-master-empty">
          <strong>{{ sources.length ? '没有匹配的来源' : '还没有来源' }}</strong>
          <p v-if="sources.length">尝试其他关键词、状态或分组。</p>
          <KButton rounded inline clear class="gap-2" v-if="filtersActive" type="button" @click="clearFilters">清除筛选</KButton>
        </div>

        <footer v-if="batchMode" class="source-master-footer">
          <label class="source-batch-select-all" title="全选当前筛选结果">
            <KCheckbox component="span" :checked="allFilteredSelected" :disabled="!filteredSources.length || toolbarBusy || editorOpen"
              aria-label="全选当前筛选结果" @change="toggleFilteredSelection"  />全选
          </label>
          <span class="source-selected-summary">已选 {{ selectedIds.length }}
            <KButton rounded inline clear class="gap-2" v-if="selectedIds.length" type="button" :disabled="toolbarBusy"
              @click="emit('selection-change', [])">取消选择</KButton>
          </span>
        </footer>
        <div v-if="batchMode && selectedIds.length" class="source-master-batch-actions">
          <div class="source-batch-buttons">
            <KButton rounded inline clear class="gap-2" type="button" :disabled="writeLocked" @click="emit('set-enabled', true)">启用</KButton>
            <KButton rounded inline clear class="gap-2" type="button" :disabled="writeLocked" @click="emit('set-enabled', false)">停用</KButton>
            <KButton rounded inline clear class="gap-2" type="button" :disabled="toolbarBusy" @click="emit('export')">导出</KButton>
            <KButton rounded inline type="button" class="k-color-brand-danger gap-2 " :disabled="writeLocked" @click="removeSelected">移除</KButton>
          </div>
          <div class="source-master-batch-group">
            <KListInput class="my-0!" outline component="div" maxlength="100" placeholder="分组名称" aria-label="所选来源的分组名称" :disabled="writeLocked" :value="batchGroup" @input="batchGroup = ($event.target as HTMLInputElement).value" />
            <KButton rounded inline clear class="gap-2" type="button" title="设置分组" aria-label="设置分组" :disabled="writeLocked||!batchGroup.trim()"
              @click="emit('set-group', batchGroup)">应用</KButton>
            <KButton rounded inline clear type="button" class="source-group-clear gap-2" :disabled="writeLocked" @click="emit('set-group', null)">清空分组</KButton>
          </div>
        </div>
      </aside>

      <slot name="editor"></slot>
    </div>
  </section>
</template>


<style scoped>
@layer components {
.source-manager-page{display:flex;flex:1 1 auto;flex-direction:column;min-width:0;min-height:0;width:100%;height:100%;max-width:none;margin:0;padding:0}
.source-manager-page:not(.source-manager-page--editing){overflow:hidden}

.source-header-import{display:flex;align-items:center;gap:5px;min-height:40px;padding:0 12px}
.source-header-import svg{width:17px;height:17px}
.source-header-add{position:relative}
.source-recovery-notice { margin-bottom:12px; border:1px solid var(--app-warning-line); border-radius:10px; padding:12px; color:var(--app-warning); background:var(--app-panel-soft); font-size:var(--ui-font-caption); }
.source-filter-summary { display:flex; align-items:center; justify-content:space-between; padding:8px 3px 10px; color:var(--app-subtle); font-size:var(--ui-font-caption); }
.source-filter-summary small { color:var(--app-subtle); font-size:var(--ui-font-caption); }
.source-master-empty { align-content:center; justify-items:center; gap:10px; padding:18px; }
.source-master-empty strong { color:var(--app-text); font-size:var(--ui-font-body); }
.source-master-empty p { margin:0; line-height:1.5; }
.source-master-empty button { min-height:36px; padding:0 12px; }
.source-selected-summary { display:flex; align-items:center; gap:8px; white-space:nowrap; }

.source-workbench { min-width:0;min-height:400px;height:calc(100dvh - 64px - var(--safe-top));display:grid;grid-template-columns:300px minmax(0,1fr);overflow:hidden;border:0;border-radius:0;background:var(--app-background); }
.source-manager-page:not(.source-manager-page--editing) .source-workbench{flex:1 1 auto;min-height:0;height:auto}
.source-manager-page--editing .source-workbench{flex:1 1 auto;height:auto;min-height:0;border-top:1px solid var(--app-line)}
.source-manager-page--editing .source-master{border-right:1px solid var(--app-line);background:var(--app-background)}
.source-manager-page--editing .source-master-footer button{pointer-events:none}
.source-master { min-width:0; display:flex; flex-direction:column; overflow:hidden; padding:14px 14px calc(14px + var(--ui-playback-inset)); border-right:1px solid var(--app-line); background:var(--app-background); }
.source-filter-menu>button, .source-toolbar-actions-menu>button{min-height:36px;padding:0 9px;text-align:left}
.source-filter-menu>button.active{color:var(--app-text);font-weight:600}
.source-filter-menu>button{overflow-wrap:anywhere}
.source-filter-menu>button:focus-visible{outline:2px solid var(--app-accent);outline-offset:-2px}
.source-filter-divider{height:1px;margin:4px 2px;background:var(--app-line)}
.source-master-list { min-height:0; flex:1; margin-right:-14px; padding-right:14px; overflow:auto; }
.source-master-empty { flex:1; display:grid; place-items:center; color:var(--app-subtle); font-size:var(--ui-font-caption); text-align:center; }
.source-master-footer { display:flex; align-items:center; justify-content:space-between; gap:8px; padding-top:8px; border-top:1px solid var(--app-line); }
.source-master-footer button { min-height:var(--ui-compact-control-height); padding:0 4px; }
.source-master-footer span { color:var(--app-muted); font-size:var(--ui-font-caption); }

@media(max-width:860px){
  .source-manager-page--editing > :deep(.subpage-header), .source-manager-page--editing .source-recovery-notice, .source-manager-page--editing .source-master{display:none}
  .source-manager-page--editing .source-workbench{display:flex;flex:1 1 auto;height:auto;min-height:0;overflow:hidden;border:0}
  .source-manager-page--editing :deep(.source-edit-screen){width:100%;min-width:0}
  .source-manager-page:not(.source-manager-page--editing) .source-workbench{height:calc(100dvh - 64px - var(--safe-top));grid-template-columns:1fr}
  .source-manager-page:not(.source-manager-page--editing) .source-master{border:0}
  .source-master{padding:16px var(--ui-page-gutter) calc(16px + var(--ui-playback-inset))}
}
.source-master-batch-actions{display:grid;gap:8px;padding-top:8px}
.source-batch-buttons{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:6px}
.source-master-batch-actions button{min-height:36px;padding:0 8px;white-space:nowrap}
.source-master-batch-actions button.danger{color:var(--app-danger);border-color:var(--app-danger-line)}
.source-master-batch-group{display:grid;grid-template-columns:minmax(0,1fr) auto auto;gap:6px}
.source-master-batch-group .k-list-input{min-width:0;width:100%}
.source-master-batch-actions .source-group-clear{padding:0 4px}
.source-master-footer button:focus-visible, .source-master-batch-actions button:focus-visible{outline:2px solid var(--app-accent);outline-offset:2px}
}
</style>

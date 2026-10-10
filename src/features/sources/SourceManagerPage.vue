<script setup lang="ts">
// 书源页管理本地筛选和选择视图；保存、登录、导入等副作用交给应用层处理。
import { computed, ref } from "vue";
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
  searchBusy: boolean;
  editorOpen?: boolean;
  editingSourceId?: string;
}>();

const emit = defineEmits<{
  back: [];
  "selection-change": [ids: string[]];
  "toggle-source": [id: string];
  "change-enabled": [source: SourceMetadata, enabled: boolean];
  "create-open": [];
  "import-local": [];
  "import-url-open": [];
  refresh: [];
  login: [];
  debug: [];
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
const activeSourceId = ref("");
const mobileInspectorOpen = ref(false);
const batchMode = ref(false);
const addMenuOpen = ref(false);
const openSourceMenuId = ref<string | null>(null);
const toolbarBusy = computed(() => props.batchBusy || props.exportBusy);
const writeLocked = computed(() => toolbarBusy.value || props.locked);

const groups = computed(() => [...new Set(props.sources.map((source) => source.group).filter((group): group is string => Boolean(group)))].sort());
const enabledCount = computed(() => props.sources.filter((source) => source.enabled).length);
const sourceFiltersActive = computed(() => Boolean(groupFilter.value || statusFilter.value !== "all"));
const sourceCapabilities = [
  { key: "search", icon: "search", label: "搜索" },
  { key: "detail", icon: "info", label: "详情" },
  { key: "toc", icon: "list", label: "目录" },
  { key: "content", icon: "file", label: "正文" },
] as const;
function sourceKindLabel(source: SourceMetadata): string {
  if (source.isRss) return "RSS";
  if (source.mediaType === "audio") return "音频";
  if (source.mediaType === "video") return "视频";
  return "书籍";
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
const allFilteredSelected = computed(() => filteredSources.value.length > 0
  && filteredSources.value.every((source) => props.selectedIds.includes(source.id)));
const activeSource = computed(() =>
  filteredSources.value.find((source) => source.id === activeSourceId.value)
  ?? filteredSources.value[0]
  ?? null);

function activateSource(source: SourceMetadata): void {
  activeSourceId.value = source.id;
  emit("edit", source);
}

function runActiveAction(action: "login" | "debug" | "export"): void {
  const source = activeSource.value;
  if (!source || writeLocked.value) return;
  emit("selection-change", [source.id]);
  if (action === "login") emit("login");
  else if (action === "debug") emit("debug");
  else emit("export");
}

function removeActiveSource(): void {
  const source = activeSource.value;
  if (!source || writeLocked.value) return;
  emit("remove", [source.id]);
}

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
  if (writeLocked.value || props.editorOpen) return;
  if (action === "new") emit("create-open");
  else if (action === "file") emit("import-local");
  else emit("import-url-open");
}

useOutsidePointerDismiss(
  () => addMenuOpen.value || !!openSourceMenuId.value || filterMenuOpen.value || actionsMenuOpen.value,
  (target) => target instanceof Element
    && !!target.closest(".source-header-add, .managed-list-toolbar__control, .managed-list-row__menu-wrap"),
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
    <SubpageHeader title="内容引擎" back-label="返回我的" @back="emit('back')">
      <template #actions>
        <div class="source-header-add">
          <button type="button" class="source-header-import"
            :disabled="writeLocked || editorOpen"
            @click="addMenuOpen = !addMenuOpen; openSourceMenuId = null; filterMenuOpen = false; actionsMenuOpen = false"><PrototypeIcon name="plus"/><span>添加</span></button>
          <div v-if="addMenuOpen" class="source-add-menu">
            <button type="button" @click="chooseAddAction('new')">新建来源</button>
            <button type="button" @click="chooseAddAction('file')">本地 JSON 导入</button>
            <button type="button" @click="chooseAddAction('url')">网络链接导入</button>
          </div>
        </div>
      </template>
    </SubpageHeader>
    <div v-if="locked" class="source-recovery-notice">资源恢复尚未完成。修改来源的操作暂时不可用，请重启应用完成恢复。</div>

    <div class="source-workbench" :class="{ 'inspector-open': mobileInspectorOpen, 'is-editing': editorOpen }">
      <aside class="source-master">
        <div class="source-filter-summary">
          <span>{{ enabledCount }} 个启用</span><small>{{ sources.length }} 个配置</small>
        </div>
        <ManagedListToolbar
          v-model:query="query"
          search-placeholder="搜索名称、分组或标识"
          :filter-active="sourceFiltersActive"
          :filter-expanded="filterMenuOpen"
          :actions-expanded="actionsMenuOpen"
          @toggle-filter="filterMenuOpen = !filterMenuOpen; actionsMenuOpen = false; addMenuOpen = false; openSourceMenuId = null"
          @toggle-actions="actionsMenuOpen = !actionsMenuOpen; filterMenuOpen = false; addMenuOpen = false; openSourceMenuId = null"
        >
          <template #filter-menu>
            <div v-if="filterMenuOpen" class="source-filter-menu">
              <button type="button" :class="{ active: statusFilter === 'all' }"
                @click="statusFilter = 'all'; filterMenuOpen = false">全部来源</button>
              <button type="button" :class="{ active: statusFilter === 'enabled' }"
                @click="statusFilter = 'enabled'; filterMenuOpen = false">仅启用</button>
              <button type="button" :class="{ active: statusFilter === 'disabled' }"
                @click="statusFilter = 'disabled'; filterMenuOpen = false">仅停用</button>
              <label><span>按分组筛选</span><select v-model="groupFilter" @change="filterMenuOpen = false">
                <option value="">全部分组</option>
                <option v-for="group in groups" :key="group" :value="group">{{ group }}</option>
              </select></label>
            </div>
          </template>
          <template #actions-menu>
            <div v-if="actionsMenuOpen" class="source-toolbar-actions-menu">
              <button type="button" :disabled="writeLocked || editorOpen"
                @click="actionsMenuOpen = false; emit('refresh')">刷新</button>
              <button type="button" :disabled="toolbarBusy || editorOpen"
                @click="batchMode = !batchMode; emit('selection-change', []); actionsMenuOpen = false">
                {{ batchMode ? '退出批量管理' : '批量管理' }}
              </button>
            </div>
          </template>
        </ManagedListToolbar>
        <div v-if="filteredSources.length" class="source-master-list" @scroll.passive="filterMenuOpen = false; actionsMenuOpen = false; openSourceMenuId = null">
          <ManagedListRow v-for="source in filteredSources" :key="source.id"
            :name="source.name" :subtitle="`${sourceKindLabel(source)} · ${source.group || '未分组'}`"
            :enabled="source.enabled"
            :active="editorOpen ? editingSourceId === source.id : activeSource?.id === source.id"
            :busy="writeLocked" :batch-mode="batchMode" :selected="selectedIds.includes(source.id)"
            :menu-open="openSourceMenuId === source.id"
            @open="editorOpen ? emit('switch-editor', source) : activateSource(source)"
            @select="emit('toggle-source', source.id)"
            @toggle-menu="openSourceMenuId = openSourceMenuId === source.id ? null : source.id; addMenuOpen = false; filterMenuOpen = false; actionsMenuOpen = false"
          >
            <template #menu>
                <button type="button" :disabled="writeLocked"
                  @click="openSourceMenuId = null; emit('change-enabled', source, !source.enabled)">{{ source.enabled ? '停用' : '启用' }}</button>
                <button type="button" class="is-danger" :disabled="writeLocked"
                  @click="openSourceMenuId = null; emit('remove', [source.id])">删除来源</button>
            </template>
          </ManagedListRow>
        </div>
        <div v-else class="source-master-empty">
          <strong>{{ sources.length ? '没有匹配的来源' : '还没有来源' }}</strong>
          <p v-if="sources.length">尝试其他关键词、状态或分组。</p>
          <button v-if="filtersActive" type="button" @click="clearFilters">清除筛选</button>
        </div>

        <footer v-if="batchMode" class="source-master-footer">
          <button type="button" :disabled="!filteredSources.length || toolbarBusy" @click="toggleFilteredSelection">{{ allFilteredSelected ? '取消选择当前结果' : '选择当前结果' }}</button>
          <span v-if="selectedIds.length" class="source-selected-summary">已选 {{ selectedIds.length }}<button type="button" :disabled="toolbarBusy" @click="emit('selection-change', [])">清空</button></span>
          <span v-else>{{ filteredSources.length }} 个来源</span>
        </footer>
        <div v-if="batchMode && selectedIds.length" class="source-master-batch-actions">
          <div>
            <button type="button" :disabled="writeLocked" @click="emit('set-enabled', true)">启用</button>
            <button type="button" :disabled="writeLocked" @click="emit('set-enabled', false)">停用</button>
            <button type="button" :disabled="toolbarBusy" @click="emit('export')">导出</button>
            <button type="button" class="danger" :disabled="writeLocked" @click="removeSelected">移除</button>
          </div>
          <div class="source-master-batch-group">
            <input v-model="batchGroup" maxlength="100" placeholder="分组名称"
              :disabled="writeLocked"/>
            <button type="button" :disabled="writeLocked||!batchGroup.trim()" @click="emit('set-group', batchGroup)">设置分组</button>
            <button type="button" :disabled="writeLocked" @click="emit('set-group', null)">清空</button>
          </div>
        </div>
      </aside>

      <slot name="editor"></slot>
      <main v-if="!editorOpen && activeSource && mobileInspectorOpen" class="source-inspector">
        <header class="source-inspector-head">
          <button class="source-mobile-back" type="button" @click="mobileInspectorOpen = false"><PrototypeIcon name="left"/></button>
          <div class="source-inspector-identity">
            <span class="source-large-icon"><PrototypeIcon :name="activeSource.isRss ? 'rss' : activeSource.mediaType === 'video' ? 'play' : activeSource.mediaType === 'audio' ? 'audio' : 'book'"/></span>
            <div><strong>{{ activeSource.name }}</strong></div>
          </div>
          <label class="source-switch">
            <input type="checkbox" :checked="activeSource.enabled" :disabled="writeLocked" @change="emit('change-enabled', activeSource, ($event.target as HTMLInputElement).checked)" />
            <span></span>
          </label>
        </header>

        <p class="source-inspector-status" :class="{ off: !activeSource.enabled }"><span class="source-dot" :class="{ off: !activeSource.enabled }"></span>{{ activeSource.enabled ? '此来源已启用' : '此来源已停用，不会参与新的搜索' }}</p>
        <div class="source-inspector-actions">
          <button type="button" :disabled="writeLocked || searchBusy || !activeSource.enabled || activeSource.isRss === true" @click="runActiveAction('debug')"><PrototypeIcon name="debug"/><span>测试</span></button>
          <button v-if="!activeSource.isRss" type="button" :disabled="writeLocked" @click="runActiveAction('login')"><PrototypeIcon name="login"/><span>登录</span></button>
          <button type="button" :disabled="writeLocked" @click="emit('edit', activeSource)"><PrototypeIcon name="edit"/><span>编辑</span></button>
          <button type="button" :disabled="toolbarBusy" @click="runActiveAction('export')"><PrototypeIcon name="export"/><span>导出</span></button>
        </div>

        <section class="source-inspector-section">
          <small>来源信息</small>
          <div class="source-facts">
            <span><small>类型</small><strong>{{ sourceKindLabel(activeSource) }}</strong></span>
            <span><small>分组</small><strong>{{ activeSource.group || '未分组' }}</strong></span>
            <span><small>状态</small><strong>{{ activeSource.enabled ? '已启用' : '已停用' }}</strong></span>
            <span><small>User-Agent</small><strong>{{ activeSource.userAgentOverride ? '自定义' : '默认' }}</strong></span>
          </div>
        </section>

        <section class="source-inspector-section">
          <small>解析能力</small>
          <div class="source-capabilities">
            <div v-for="item in sourceCapabilities" :key="item.key" :class="{ configured: !!activeSource.capabilities?.[item.key] }">
              <PrototypeIcon :name="item.icon"/>
              <strong>{{ item.label }}</strong>
              <small>{{ activeSource.capabilities?.[item.key] ? '已配置' : '未配置' }}</small>
            </div>
          </div>
        </section>
        <section class="source-inspector-section">
          <small>运行验证</small>
          <div class="source-verification-state">
            <span class="source-verification-icon"><PrototypeIcon name="info"/></span>
            <div><strong>查看真实解析结果</strong><small>字段已配置不代表网站请求成功，点击测试验证实际规则。</small></div>
            <button type="button" :disabled="writeLocked || searchBusy || !activeSource.enabled || activeSource.isRss === true" @click="runActiveAction('debug')">运行测试</button>
          </div>
        </section>
        <section v-if="selectedIds.length > 1" class="source-inspector-section source-batch-panel">
          <small>批量操作 · 已选 {{ selectedIds.length }}</small>
          <div class="source-batch-group-row">
            <input v-model="batchGroup" maxlength="100" placeholder="分组名称" :disabled="writeLocked" />
            <button type="button" :disabled="writeLocked || !batchGroup.trim()" @click="emit('set-group', batchGroup)">应用分组</button>
            <button type="button" :disabled="writeLocked" @click="emit('set-group', null)">清除分组</button>
          </div>
          <div class="source-batch-actions">
            <button type="button" :disabled="writeLocked" @click="emit('set-enabled', true)">批量启用</button>
            <button type="button" :disabled="writeLocked" @click="emit('set-enabled', false)">批量停用</button>
            <button type="button" :disabled="toolbarBusy" @click="emit('export')">{{ exportBusy ? '导出中…' : '导出所选' }}</button>
            <button type="button" class="danger" :disabled="writeLocked" @click="removeSelected">移除所选</button>
          </div>
        </section>

        <button type="button" class="source-danger-row" :disabled="writeLocked" @click="removeActiveSource">删除这个来源</button>
      </main>

    </div>
  </section>
</template>


<style scoped>
.source-manager-page{display:flex;flex:1 1 auto;flex-direction:column;min-width:0;min-height:0;width:100%;height:100%;max-width:none;margin:0;padding:0}
.source-manager-page:not(.source-manager-page--editing){overflow:hidden}
.source-manager-page > :deep(.subpage-header){position:sticky;top:0;z-index:10;box-sizing:border-box;min-height:64px;margin:0;padding:4px max(18px,calc((100% - 1180px)/2));border-bottom:1px solid var(--app-line);background:#f6f7f9}
.source-manager-page :deep(.subpage-header__copy h2){color:#22252d;font-size:14px;line-height:1.25}

.source-header-import{display:flex;align-items:center;gap:5px;min-height:40px;padding:0 12px;border:0;border-radius:10px;background:var(--app-accent);color:#fff;font-size:11px;font-weight:700;cursor:pointer}
.source-header-import svg{width:17px;height:17px}
.source-header-import:disabled{opacity:.5;cursor:default}
.source-header-add{position:relative}
.source-add-menu{position:absolute;right:0;top:calc(100% + 7px);z-index:55;display:grid;gap:3px;width:232px;padding:7px;border:1px solid var(--app-line);border-radius:11px;background:#fff;box-shadow:0 16px 40px #20203322}
.source-add-menu button{display:flex;align-items:center;min-height:40px;padding:9px;border:0;border-radius:8px;background:transparent;color:#343640;text-align:left;font-size:11px;font-weight:650;cursor:pointer}
.source-add-menu button:hover{background:#f2efff}
.source-add-menu button:disabled{opacity:.5;cursor:default}
.source-recovery-notice { margin-bottom:12px; border:1px solid #eedfbf; border-radius:10px; padding:12px; color:#81664b; background:#fff8ed; font-size:13px; }
.source-filter-summary { display:flex; align-items:center; justify-content:space-between; padding:8px 3px 10px; color:#979ba5; font-size:9px; }
.source-filter-summary small { color:#979ba5; font-size:9px; }
.source-master-empty { align-content:center; justify-items:center; gap:10px; padding:18px; }
.source-master-empty strong { color:#444752; font-size:14px; }
.source-master-empty p { margin:0; line-height:1.5; }
.source-master-empty button { min-height:36px; padding:0 12px; border:0; border-radius:9px; color:var(--app-accent); background:var(--app-accent-soft); cursor:pointer; }
.source-selected-summary { display:flex; align-items:center; gap:7px; }
.source-selected-summary button { color:var(--app-accent); background:none; }
.source-inspector-status { display:inline-flex; align-items:center; gap:7px; margin:13px 0 0; color:var(--app-accent); font-size:12px; }
.source-inspector-status.off { color:#9296a0; }

.source-workbench { min-width:0;min-height:400px;height:calc(100dvh - 64px);display:grid;grid-template-columns:300px minmax(0,1fr);overflow:hidden;border:0;border-radius:0;background:#fff; }
.source-manager-page:not(.source-manager-page--editing) .source-workbench{flex:1 1 auto;min-height:0;height:auto}
.source-manager-page--editing .source-workbench{flex:1 1 auto;height:auto;min-height:0;border-top:1px solid var(--app-line)}
.source-manager-page--editing .source-master{border-right:1px solid var(--app-line);background:#fafafd}
.source-manager-page--editing .source-master-footer button{pointer-events:none;opacity:.4}
.source-master { min-width:0; display:flex; flex-direction:column; overflow:hidden; padding:14px; border-right:1px solid #e5e7ee; background:#fafafd; }
.source-filter-menu,.source-toolbar-actions-menu{position:absolute;right:0;top:calc(100% + 5px);z-index:70;display:grid;gap:3px;min-width:176px;padding:7px;border:1px solid var(--app-line);border-radius:11px;background:#fff;box-shadow:0 16px 40px #20203322}
.source-filter-menu>button,.source-toolbar-actions-menu>button{min-height:36px;padding:0 9px;border:0;border-radius:8px;background:transparent;color:#343640;text-align:left;font-size:11px;cursor:pointer}
.source-filter-menu>button:hover,.source-toolbar-actions-menu>button:hover:not(:disabled){background:#f2efff}
.source-filter-menu>button.active{color:#564bb4;background:#f2efff}
.source-filter-menu>label{display:grid;gap:5px;padding:8px 9px 4px;border-top:1px solid #eceef2;color:#808491;font-size:10px}
.source-filter-menu select{height:32px;padding:0 7px;border:1px solid var(--app-line);border-radius:8px;color:#555968;background:#fff;font-size:11px}
.source-toolbar-actions-menu>button:disabled{opacity:.5;cursor:default}
.source-master-list { min-height:0; flex:1; margin-right:-14px; padding-right:14px; overflow:auto; }
.source-dot { width:7px; height:7px; border-radius:50%; background:#5e8b6b; }
.source-dot.off { background:#c7cad0; }
.source-master-empty { flex:1; display:grid; place-items:center; color:#9296a3; font-size:12px; text-align:center; }
.source-master-footer { display:flex; align-items:center; justify-content:space-between; gap:8px; padding-top:10px; border-top:1px solid var(--app-line); }
.source-master-footer button { min-height:34px; padding:0 8px; border:0; border-radius:8px; color:var(--app-accent); background:#f0edff; font-size:11px; cursor:pointer; }
.source-master-footer span { color:#9599a4; font-size:11px; }

.source-inspector { min-width:0; overflow:auto; padding:30px 34px; }
.source-inspector-head { display:grid; grid-template-columns:minmax(0,1fr) auto; align-items:center; gap:14px; }
.source-inspector-identity { min-width:0; display:flex; align-items:center; gap:12px; }
.source-large-icon { width:46px; height:46px; flex:none; display:grid; place-items:center; border-radius:13px; color:#5c519f; background:#eeeafe; font-size:18px; font-weight:800; }
.source-inspector-identity div { min-width:0; }
.source-inspector-identity strong,.source-inspector-identity small { display:block; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.source-inspector-identity strong { color:#333743; font-size:20px; }
.source-inspector-identity small { margin-top:4px; color:#9499a4; font-size:11px; }
.source-switch { position:relative; width:40px; height:24px; }
.source-switch input { position:absolute; opacity:0; pointer-events:none; }
.source-switch span { position:absolute; inset:0; border-radius:999px; background:#d3d8d4; cursor:pointer; }
.source-switch span::after { position:absolute; top:3px; left:3px; width:18px; height:18px; border-radius:50%; background:#fff; box-shadow:0 1px 4px #0002; content:""; transition:transform .15s; }
.source-switch input:checked + span { background:var(--app-accent); }
.source-switch input:checked + span::after { transform:translateX(16px); }
.source-inspector-actions { display:flex; flex-wrap:wrap; gap:7px; margin:22px 0; }
.source-inspector-actions button { min-height:36px; display:flex; align-items:center; gap:6px; padding:0 11px; border:0; border-radius:9px; color:#676a79; background:#efeff3; font-size:13px; cursor:pointer; }
.source-inspector-actions button span { font-size:12px; }
.source-inspector-section { padding:20px 0; border-top:1px solid var(--app-line); }
.source-inspector-section > small { display:block; margin-bottom:10px; color:#9499a4; font-size:11px; }
.source-facts { display:grid; grid-template-columns:repeat(4,minmax(0,1fr)); gap:8px; }
.source-facts > span { min-width:0; padding:12px; border-radius:11px; background:#f3f2fa; }
.source-facts small,.source-facts strong { display:block; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.source-facts small { color:#969aa6; font-size:10px; }
.source-facts strong { margin-top:4px; color:#434656; font-size:12px; }
.source-batch-group-row { display:grid; grid-template-columns:minmax(0,1fr) auto auto; gap:7px; }
.source-batch-group-row input { min-width:0; height:38px; padding:0 9px; border:1px solid var(--app-line); border-radius:8px; }
.source-batch-group-row button,.source-batch-actions button { min-height:38px; padding:0 10px; border:0; border-radius:8px; color:#666a7b; background:#f0edff; font-size:11px; cursor:pointer; }
.source-batch-actions { display:flex; flex-wrap:wrap; gap:7px; margin-top:8px; }
.source-batch-actions .danger { color:#a75048; background:#fff0ef; }
.source-danger-row { width:100%; min-height:44px; margin-top:6px; border:0; border-radius:9px; color:#ad554d; background:#fff2f1; text-align:left; padding:0 12px; cursor:pointer; }
.source-mobile-back { display:none; }

@media(max-width:860px){
  .source-manager-page > :deep(.subpage-header){padding-right:14px;padding-left:14px}
  .source-manager-page--editing > :deep(.subpage-header),
  .source-manager-page--editing .source-recovery-notice,
  .source-manager-page--editing .source-master{display:none}
  .source-manager-page--editing .source-workbench{display:block;height:auto;min-height:100dvh;overflow:visible;border:0}
  .source-manager-page--editing :deep(.source-edit-screen){width:100%;min-width:0}
  .source-manager-page:not(.source-manager-page--editing) .source-workbench{height:calc(100dvh - 64px);grid-template-columns:1fr}
  .source-manager-page:not(.source-manager-page--editing) .source-master{border:0}
  .source-master{padding:12px 14px}
}

@media(max-width:760px) {
  .source-master { border-right:0; }
  .source-inspector { display:none; padding:20px 16px; }
  .source-workbench.inspector-open .source-master { display:none; }
  .source-workbench.inspector-open .source-inspector { display:block; }
  .source-inspector-head { grid-template-columns:36px minmax(0,1fr) auto; }
  .source-mobile-back { width:34px; height:34px; display:grid; place-items:center; border:0; border-radius:9px; color:#6e7282; background:#f0edff; }
  .source-mobile-back svg { width:19px;height:19px; }
  .source-inspector-identity strong { font-size:17px; }
  .source-facts { grid-template-columns:repeat(2,minmax(0,1fr)); }
  .source-batch-group-row { grid-template-columns:1fr 1fr; }
  .source-batch-group-row input { grid-column:1/-1; }
}

.source-inspector-actions svg{width:19px;height:19px;stroke-linecap:round;stroke-linejoin:round}
.source-large-icon svg{width:24px;height:24px}
.source-capabilities{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:9px;margin-top:12px}
.source-capabilities>div{display:flex;flex-direction:column;align-items:center;justify-content:center;gap:6px;min-height:94px;border:1px solid #e7e8ed;border-radius:11px;background:#f7f8fa;color:#9da0aa;text-align:center}
.source-capabilities>div.configured{color:var(--app-accent);background:#f2effe;border-color:#e1dafb}
.source-capabilities svg{width:21px;height:21px}
.source-capabilities strong{font-size:12px;color:#424550}
.source-capabilities small{font-size:10px;color:#9599a4}
.source-verification-state{display:grid;grid-template-columns:37px minmax(0,1fr) auto;gap:11px;align-items:center;margin-top:10px;padding:13px;border:1px solid var(--app-line);border-radius:12px;background:#fafafd}
.source-verification-icon{display:grid;place-items:center;width:36px;height:36px;border-radius:10px;background:#efeff8;color:#7d80a0}
.source-verification-state>div{display:grid;gap:4px;min-width:0}
.source-verification-state strong{font-size:12px;color:#555965}
.source-verification-state small{font-size:11px;line-height:1.5;color:#9599a3}
.source-verification-state button{min-height:34px;padding:5px 9px;border:1px solid #dfdaf5;border-radius:9px;background:#f3efff;color:var(--app-accent);cursor:pointer;font-size:11px}
.source-verification-state button:disabled{opacity:.45;cursor:default}
@media(max-width:700px){.source-capabilities{grid-template-columns:repeat(2,minmax(0,1fr))}.source-verification-state{grid-template-columns:36px minmax(0,1fr)}.source-verification-state button{grid-column:2;justify-self:start}}
.source-master-batch-actions{display:grid;gap:7px;padding-top:10px;border-top:1px solid var(--app-line)}
.source-master-batch-actions>div{display:flex;flex-wrap:wrap;gap:4px}
.source-master-batch-actions button{min-height:29px;padding:0 8px;border:0;border-radius:7px;background:#efebff;color:#6458a8;font-size:10px;cursor:pointer}
.source-master-batch-actions button.danger{color:#a7535b;background:#fff0f1}
.source-master-batch-actions button:disabled{opacity:.5;cursor:default}
.source-master-batch-group input{min-width:0;flex:1;padding:0 7px;border:1px solid #e2e3eb;border-radius:7px;font-size:10px}
</style>

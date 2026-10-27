<script setup lang="ts">
import { ListInput as KListInput, Button as KButton } from "konsta/vue";
// 书架整理视图处理排序、筛选和选择交互，不直接执行书架事务。
import { ref } from "vue";

withDefaults(defineProps<{
  groups: string[];
  activeGroup: string;
  manageGroupsOpen?: boolean;
}>(), { groups: () => [], activeGroup: "", manageGroupsOpen: false });

const emit = defineEmits<{
  filterGroup: [group: string];
  createGroup: [name: string];
  renameGroup: [oldName: string, newName: string];
  deleteGroup: [name: string];
}>();

const editingName = ref("");
const renamingGroup = ref<string | null>(null);
const newGroupName = ref("");

function createGroup(): void {
  const name = newGroupName.value.trim();
  if (!name) return;
  emit("createGroup", name);
  newGroupName.value = "";
}

function rename(oldName: string): void {
  const next = editingName.value.trim();
  if (!next || next === oldName) {
    renamingGroup.value = null;
    return;
  }
  emit("renameGroup", oldName, next);
  renamingGroup.value = null;
}
</script>

<template>
  <section class="shelf-organizer">
    <div class="organizer-main">
      <div class="group-strip">
        <KButton rounded :tonal="!activeGroup" :clear="!(!activeGroup)" inline class="group-pill gap-2" :class="{ selected: !activeGroup }" @click="emit('filterGroup', '')">全部藏书</KButton>
        <KButton rounded :tonal="activeGroup === group" :clear="!(activeGroup === group)" inline v-for="group in groups" :key="group" class="group-pill gap-2" :class="{ selected: activeGroup === group }" @click="emit('filterGroup', activeGroup === group ? '' : group)">{{ group }}</KButton>
      </div>
    </div>
    <div v-if="manageGroupsOpen" class="group-manager">
      <div class="group-manager-title"><strong>整理书架分组</strong></div>
      <div v-if="groups.length" class="managed-groups">
        <div v-for="group in groups" :key="group" class="managed-group-row">
          <template v-if="renamingGroup === group"><KListInput component="div" @keydown.enter.prevent="rename(group)" @keydown.esc="renamingGroup = null" :value="editingName" @input="editingName = ($event.target as HTMLInputElement).value" /><KButton rounded inline clear class="gap-2" @click="rename(group)">保存</KButton>
<KButton rounded inline clear class="gap-2" @click="renamingGroup = null">取消</KButton></template>
          <template v-else><span>{{ group }}</span><KButton rounded inline clear class="gap-2" @click="renamingGroup = group; editingName = group">重命名</KButton>
<KButton rounded inline clear class="remove-group k-color-brand-danger gap-2 " @click="emit('deleteGroup', group)">删除</KButton></template>
        </div>
      </div>
      <div v-else class="no-groups">还没有分组，可以先创建一个。</div>
      <form class="new-group-form" @submit.prevent="createGroup"><KListInput component="div" maxlength="80" placeholder="新分组名称" :value="newGroupName" @input="newGroupName = ($event.target as HTMLInputElement).value" /><KButton rounded inline clear class="gap-2" type="submit" :disabled="!newGroupName.trim()">创建分组</KButton></form>
    </div>
  </section>
</template>

<style scoped>
@layer components {
.shelf-organizer { margin:0 0 14px; border-bottom:1px solid var(--app-line); }
.organizer-main { display:flex; min-height:48px; align-items:center; }
.group-strip { min-width:0; display:flex; align-items:center; gap:6px; overflow-x:auto; scrollbar-width:none; }
.group-strip::-webkit-scrollbar { display:none; }
.group-pill { flex:none; min-height:44px; padding:7px 12px; }
.group-manager { display:grid; grid-template-columns:minmax(170px,.65fr) minmax(0,1.35fr) minmax(190px,.8fr); align-items:start; gap:14px; margin:0 0 12px; padding:12px 14px; border:1px solid var(--app-line); border-radius:10px; background:var(--app-surface); }
.group-manager-title { display:flex; flex-direction:column; gap:5px; }.group-manager-title strong { color:var(--app-text); font-size:var(--ui-font-heading); }.group-manager-title span, .no-groups { color:var(--app-muted); font-size:var(--ui-font-body); line-height:1.5; }
.managed-groups { display:flex; flex-wrap:wrap; gap:7px; }.managed-group-row { display:flex; align-items:center; gap:5px; padding:3px 5px 3px 10px; border-radius:7px; background:var(--app-panel-soft); color:var(--app-text); font-size:var(--ui-font-body); }.managed-group-row button, .new-group-form button { min-height:40px; padding:0 9px; }.managed-group-row .k-list-input, .new-group-form .k-list-input { min-width:0; width:110px; }
.new-group-form { display:flex; align-items:center; gap:6px; }.new-group-form .k-list-input { flex:1; width:unset; }.new-group-form button { flex:none; min-height:44px; }.new-group-form button:disabled { opacity:.55; cursor:not-allowed; }
@media(max-width:760px) { .organizer-main { min-height:unset; align-items:flex-start; padding:0; }.group-strip { width:100%; }.group-manager { grid-template-columns:1fr; gap:9px; }.new-group-form .k-list-input { width:100%; } }
}
</style>

<script setup lang="ts">
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
        <button class="group-pill" :class="{ selected: !activeGroup }" @click="emit('filterGroup', '')">全部藏书</button>
        <button v-for="group in groups" :key="group" class="group-pill" :class="{ selected: activeGroup === group }" @click="emit('filterGroup', activeGroup === group ? '' : group)">{{ group }}</button>
      </div>
    </div>
    <div v-if="manageGroupsOpen" class="group-manager">
      <div class="group-manager-title"><strong>整理书架分组</strong></div>
      <div v-if="groups.length" class="managed-groups">
        <div v-for="group in groups" :key="group" class="managed-group-row">
          <template v-if="renamingGroup === group"><input v-model="editingName" @keydown.enter.prevent="rename(group)" @keydown.esc="renamingGroup = null" /><button @click="rename(group)">保存</button><button @click="renamingGroup = null">取消</button></template>
          <template v-else><span>{{ group }}</span><button @click="renamingGroup = group; editingName = group">重命名</button><button class="remove-group" @click="emit('deleteGroup', group)">删除</button></template>
        </div>
      </div>
      <div v-else class="no-groups">还没有分组，可以先创建一个。</div>
      <form class="new-group-form" @submit.prevent="createGroup"><input v-model="newGroupName" maxlength="80" placeholder="新分组名称" /><button type="submit" :disabled="!newGroupName.trim()">创建分组</button></form>
    </div>
  </section>
</template>

<style scoped>
.shelf-organizer { margin:0 0 14px; border-bottom:1px solid #e6eae6; }
.organizer-main { display:flex; min-height:48px; align-items:center; }
.group-strip { min-width:0; display:flex; align-items:center; gap:6px; overflow-x:auto; scrollbar-width:none; }
.group-strip::-webkit-scrollbar { display:none; }
.group-pill { flex:none; min-height:44px; padding:7px 12px; border:1px solid transparent; border-radius:8px; color:#627067; background:transparent; font-size:14px; cursor:pointer; }
.group-pill:hover { color:#3c725f; background:#f0f5f1; }
.group-pill.selected { color:#385f51; background:#eaf1ec; font-weight:700; }
.group-manager { display:grid; grid-template-columns:minmax(170px,.65fr) minmax(0,1.35fr) minmax(190px,.8fr); align-items:start; gap:14px; margin:0 0 12px; padding:12px 14px; border:1px solid #e0e7e0; border-radius:10px; background:#fff; }
.group-manager-title { display:flex; flex-direction:column; gap:5px; }.group-manager-title strong { color:#45554a; font-size:14px; }.group-manager-title span,.no-groups { color:#69766d; font-size:14px; line-height:1.5; }
.managed-groups { display:flex; flex-wrap:wrap; gap:7px; }.managed-group-row { display:flex; align-items:center; gap:5px; padding:3px 5px 3px 10px; border-radius:7px; background:#f2f6f2; color:#4f6255; font-size:14px; }.managed-group-row button,.new-group-form button { min-height:40px; padding:0 9px; border:0; border-radius:5px; color:#4f705a; background:transparent; font-size:14px; cursor:pointer; }.managed-group-row button:hover,.new-group-form button:hover { background:#e7f0e9; }.managed-group-row .remove-group { color:#a04c42; }.managed-group-row input,.new-group-form input { min-width:0; width:110px; min-height:42px; padding:0 9px; border:1px solid #d7e0d7; border-radius:6px; background:#fff; font-size:14px; }
.new-group-form { display:flex; align-items:center; gap:6px; }.new-group-form input { flex:1; width:unset; }.new-group-form button { flex:none; min-height:44px; color:#fff; background:#3c725f; }.new-group-form button:disabled { opacity:.55; cursor:not-allowed; }
@media(max-width:760px) { .organizer-main { min-height:unset; align-items:flex-start; padding:0; }.group-strip { width:100%; }.group-manager { grid-template-columns:1fr; gap:9px; }.new-group-form input { width:100%; } }
</style>

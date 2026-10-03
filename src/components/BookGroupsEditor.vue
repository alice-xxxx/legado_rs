<script setup lang="ts">
import { ref, watch } from "vue";

const props = defineProps<{ groups: string[]; assigned: string[] }>();
const emit = defineEmits<{ save: [groups: string[]] }>();
const selected = ref([...props.assigned]);
const newName = ref("");

watch(() => props.assigned, (value) => { selected.value = [...value]; }, { deep: true });

function toggle(name: string): void {
  selected.value = selected.value.includes(name)
    ? selected.value.filter((group) => group !== name)
    : [...selected.value, name];
  emit("save", selected.value);
}

function create(): void {
  const name = newName.value.trim();
  if (!name) return;
  if (!selected.value.includes(name)) selected.value.push(name);
  newName.value = "";
  emit("save", selected.value);
}
</script>

<template>
  <div class="book-groups-editor">
    <strong>书架分组</strong>
    <div class="group-checkboxes">
      <label v-for="group in groups" :key="group"><input type="checkbox" :checked="selected.includes(group)" @change="toggle(group)" />{{ group }}</label>
      <span v-if="!groups.length" class="no-groups-yet">暂无分组</span>
    </div>
    <form class="inline-new-group" @submit.prevent="create"><input v-model="newName" maxlength="60" aria-label="添加到新分组" placeholder="新建分组并加入此书" /><button :disabled="!newName.trim()" aria-label="创建分组">＋</button></form>
  </div>
</template>

<style scoped>
.book-groups-editor { display:grid; gap:10px; padding:15px 17px; border:1px solid #e9ede9; border-radius:10px; background:#fafbf9; }
.book-groups-editor > strong { color:#65736a; font-size:14px; }
.group-checkboxes { display:flex; flex-wrap:wrap; gap:7px; }
.group-checkboxes label { display:flex; min-height:44px; align-items:center; gap:7px; padding:5px 10px; border:1px solid #dfe6df; border-radius:7px; color:#526258; background:#fff; font-size:14px; cursor:pointer; }
.group-checkboxes input { accent-color:#3c725f; }
.no-groups-yet { color:#65736a; font-size:14px; }
.inline-new-group { display:flex; gap:6px; }.inline-new-group input { min-width:0; min-height:44px; flex:1; padding:0 10px; border:1px solid #dbe3db; border-radius:7px; background:#fff; font-size:14px; }.inline-new-group button { width:46px; min-height:44px; border:0; border-radius:7px; color:#fff; background:#3c725f; font-size:17px; cursor:pointer; }.inline-new-group button:disabled { opacity:.5; cursor:not-allowed; }
</style>

<script setup lang="ts">
import { Glass as KGlass, Button as KButton, Checkbox as KCheckbox } from "konsta/vue";
import { computed } from "vue";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";

const props = defineProps<{
  open: boolean; title: string; busy: boolean; error: string;
  entries: Array<{ key: string; name: string }>; selectedKeys: string[];
}>();
const emit = defineEmits<{
  cancel: []; save: []; retry: []; "update:selectedKeys": [keys: string[]];
}>();
const selected = computed({ get: () => props.selectedKeys, set: keys => emit("update:selectedKeys", keys) });
</script>

<template>
  <section v-if="open" class="modal-backdrop" @click.self="!busy && emit('cancel')" @keydown.esc="!busy && emit('cancel')">
    <KGlass component="article" :highlight="false" class="app-modal deep-link-import" role="dialog" aria-modal="true" aria-labelledby="deep-link-import-title">
      <header>
        <h2 id="deep-link-import-title">{{ title }}</h2>
        <KButton rounded inline clear class="size-11 p-0 gap-2" :disabled="busy" data-back-action aria-label="关闭" @click="emit('cancel')"><PrototypeIcon name="close" /></KButton>
      </header>
      <p v-if="busy && !entries.length" role="status">正在读取…</p>
      <p v-if="error" class="deep-link-import-error" role="alert">{{ error }}</p>
      <div v-if="entries.length" class="deep-link-import-list">
        <label v-for="entry in entries" :key="entry.key">
          <KCheckbox component="span" :checked="selected.includes(entry.key)" :disabled="busy"
            @change="selected = ($event.target as HTMLInputElement).checked ? [...selected, entry.key] : selected.filter(key => key !== entry.key)" />
          <span>{{ entry.name }}</span>
        </label>
      </div>
      <footer>
        <KButton rounded inline tonal small v-if="entries.length" class="gap-2" :disabled="busy"
          @click="selected = selected.length === entries.length ? [] : entries.map(entry => entry.key)">
          {{ selected.length === entries.length ? '取消全选' : '全选' }}
        </KButton>
        <div class="modal-actions">
          <KButton rounded inline tonal v-if="error && !entries.length" class="gap-2" :disabled="busy" @click="emit('retry')">重试</KButton>
          <KButton rounded inline tonal class="gap-2" :disabled="busy" @click="emit('cancel')">取消</KButton>
          <KButton rounded inline class="gap-2" :disabled="busy || !selected.length" @click="emit('save')">
            {{ busy && entries.length ? '正在导入…' : `导入（${selected.length}）` }}
          </KButton>
        </div>
      </footer>
    </KGlass>
  </section>
</template>

<style scoped>
@layer components {
.deep-link-import{display:flex;flex-direction:column;width:min(560px,100%);max-height:80dvh;padding:0;overflow:hidden}
.deep-link-import header,.deep-link-import footer{display:flex;align-items:center;justify-content:space-between;gap:12px;flex:none;padding:20px 24px}
.deep-link-import h2{margin:0;font-size:var(--ui-font-heading)}
.deep-link-import p{padding:0 24px}
.deep-link-import-list{min-height:0;overflow-y:auto;padding:0 24px}
.deep-link-import-list label{display:flex;align-items:center;gap:12px;padding:12px 0;border-bottom:1px solid var(--app-line);cursor:pointer}
.deep-link-import-list span{overflow-wrap:anywhere;min-width:0}
.deep-link-import-list input{flex:none;accent-color:var(--app-accent)}
.deep-link-import-error{color:var(--app-danger);overflow-wrap:anywhere}
.deep-link-import .modal-actions{margin:0 0 0 auto;padding:0}
}
</style>

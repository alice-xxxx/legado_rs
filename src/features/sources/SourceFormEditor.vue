<script setup lang="ts">
import { computed, nextTick, onUnmounted, ref, watch } from "vue";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import { bookSourceTypeOptions } from "./sourceTypeOptions";
import type { SourceEditorField } from "./sourceEditorFields";
import { readSourceEditorDraft, serializeSourceEditorDraft, sourceEditorTabs } from "./sourceEditorDraft";

const props = defineProps<{ jsonText: string; enabled: boolean; disabled: boolean }>();
const emit = defineEmits<{ pending: [value: boolean] }>();
const activeTab = ref("basic");
const tabs = computed(() => sourceEditorTabs(props.jsonText));
const fields = computed(() => tabs.value.flatMap(tab => tab.fields));
const currentTab = computed(() => tabs.value.find(tab => tab.id === activeTab.value) ?? tabs.value[0]!);
const draft = ref<Record<string, string>>({});
const baseline = ref<Record<string, string>>({});
const error = ref("");
const editingField = ref<SourceEditorField | null>(null);
let originalLongValue = "";
let returnFocus: HTMLElement | null = null;
let baselineJson = props.jsonText;
function reset(json = props.jsonText): void {
  baselineJson = json;
  draft.value = readSourceEditorDraft(json, sourceEditorTabs(json).flatMap(tab => tab.fields), props.enabled);
  baseline.value = { ...draft.value };
  editingField.value = null;
  error.value = "";
}
reset();
const pending = computed(() => fields.value.some(field => draft.value[field.key] !== baseline.value[field.key]));
watch(() => props.jsonText, json => reset(json));
watch(pending, value => emit("pending", value), { immediate: true, flush: "sync" });
onUnmounted(() => emit("pending", false));

function serialize(): string | null {
  try {
    const json = serializeSourceEditorDraft(baselineJson, fields.value, baseline.value, draft.value);
    error.value = "";
    return json;
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause);
    return null;
  }
}
function preview(field: SourceEditorField): string {
  return draft.value[field.key]?.replace(/\s+/g, " ").trim() || "未设置";
}
function openField(field: SourceEditorField): void {
  if (props.disabled) return;
  returnFocus = document.activeElement instanceof HTMLButtonElement ? document.activeElement : null;
  editingField.value = field;
  originalLongValue = draft.value[field.key] ?? "";
}
function closeField(discard = false): void {
  if (props.disabled) return;
  if (discard && editingField.value) draft.value[editingField.value.key] = originalLongValue;
  editingField.value = null;
  void nextTick(() => returnFocus?.focus());
}
function trapFieldFocus(event: KeyboardEvent): void {
  if (event.key !== "Tab") return;
  const controls = (event.currentTarget as HTMLElement).querySelectorAll<HTMLElement>("button:not(:disabled),textarea:not(:disabled)");
  const first = controls[0], last = controls[controls.length - 1];
  if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
  else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
}
defineExpose({ serialize, reset });
</script>

<template>
  <div class="source-field-editor" :inert="!!editingField">
    <nav class="source-field-tabs" aria-label="源配置分类" role="tablist">
      <button v-for="tab in tabs" :key="tab.id" type="button" role="tab"
        :aria-selected="currentTab.id === tab.id" :class="{ active: currentTab.id === tab.id }"
        @click="activeTab = tab.id">{{ tab.label }}</button>
    </nav>
    <div class="source-field-list" role="tabpanel" :aria-label="currentTab.label">
      <div v-for="field in currentTab.fields" :key="field.key" class="source-field"
        :class="{ 'source-field--compact': ['toggle', 'type', 'number'].includes(field.kind) }">
        <label :for="`source-field-${field.key}`" class="source-field-label">
          <span>{{ field.label }}</span><small>{{ field.key.split('.').slice(-1)[0] }}</small>
        </label>
        <button v-if="field.kind === 'long'" :id="`source-field-${field.key}`" type="button"
          class="source-field-preview" :class="{ empty: !draft[field.key] }" :disabled="disabled" @click="openField(field)">
          <span>{{ preview(field) }}</span><PrototypeIcon name="right" />
        </button>
        <label v-else-if="field.kind === 'toggle'" class="source-field-switch">
          <input :id="`source-field-${field.key}`" type="checkbox" :checked="draft[field.key] === 'true'" :disabled="disabled"
            @change="draft[field.key] = ($event.target as HTMLInputElement).checked ? 'true' : 'false'" />
          <span aria-hidden="true"></span>
        </label>
        <select v-else-if="field.kind === 'type'" :id="`source-field-${field.key}`" v-model="draft[field.key]" :disabled="disabled">
          <option v-for="option in bookSourceTypeOptions(draft[field.key] ?? '0')" :key="option.value" :value="option.value">{{ option.label }}</option>
        </select>
        <input v-else :id="`source-field-${field.key}`" v-model="draft[field.key]" :disabled="disabled"
          :inputmode="field.kind === 'number' ? 'numeric' : 'text'" type="text" spellcheck="false" autocomplete="off" />
      </div>
    </div>
    <p v-if="error" class="source-field-error" role="alert">{{ error }}</p>
    <Teleport to="body">
      <section v-if="editingField" class="source-field-overlay" role="dialog" aria-modal="true"
        aria-labelledby="source-long-field-title" @keydown="trapFieldFocus" @keydown.esc.stop.prevent="closeField(true)">
        <header>
          <button type="button" data-back-action :disabled="disabled" @click="closeField(true)">取消</button>
          <div><strong id="source-long-field-title">{{ editingField.label }}</strong><small>{{ editingField.key }}</small></div>
          <button type="button" :disabled="disabled" @click="closeField()">完成</button>
        </header>
        <textarea v-model="draft[editingField.key]" :disabled="disabled"
          :aria-label="editingField.label" spellcheck="false" autocapitalize="off" autocomplete="off" />
      </section>
    </Teleport>
  </div>
</template>

<style scoped>
.source-field-editor{min-width:0;color:var(--app-text,var(--app-text))}
.source-field-tabs{position:sticky;top:0;z-index:5;display:flex;overflow-x:auto;background:var(--app-surface);border-bottom:1px solid var(--app-line);scrollbar-width:none}
.source-field-tabs>button{flex:1 0 auto;min-height:48px;padding:0 18px;border:0;border-bottom:3px solid transparent;background:transparent;color:var(--app-muted);font:inherit;font-size:var(--ui-font-body);cursor:pointer}
.source-field-tabs>button.active{border-bottom-color:var(--app-accent);color:var(--app-accent);font-weight:650}
.source-field-list{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:20px 22px;padding:24px 0}
.source-field{display:grid;gap:8px;min-width:0;grid-column:1/-1}
.source-field--compact{grid-column:auto}
.source-field-label{display:flex;align-items:baseline;gap:8px;flex-wrap:wrap;font-size:var(--ui-font-body);line-height:1.4;color:var(--app-muted)}
.source-field-label small{font-size:var(--ui-font-caption);font-weight:400;color:var(--app-subtle);overflow-wrap:anywhere}
.source-field input:not([type=checkbox]),.source-field select,.source-field-preview{box-sizing:border-box;width:100%;min-width:0;min-height:46px;padding:11px 13px;border:1px solid var(--app-line);border-radius:8px;background:var(--app-panel-soft);color:var(--app-text);font:inherit;font-size:var(--ui-font-body);line-height:1.5}
.source-field-preview{display:flex;align-items:center;gap:12px;text-align:left;cursor:pointer}
.source-field-preview>span{flex:1;min-width:0;overflow:hidden;white-space:nowrap;text-overflow:ellipsis;font:var(--ui-font-code)/1.6 ui-monospace,SFMono-Regular,Consolas,monospace}
.source-field-preview.empty>span{color:var(--app-subtle);font-family:inherit}
.source-field-preview :deep(svg){flex:none;width:18px;height:18px;color:var(--app-subtle)}
.source-field-switch{position:relative;display:flex;align-items:center;min-height:46px;cursor:pointer;width:max-content}
.source-field-switch input{position:absolute;width:1px;height:1px;opacity:0}
.source-field-switch>span{position:relative;width:48px;height:28px;border-radius:16px;background:var(--app-accent-soft);transition:background .15s}
.source-field-switch>span::after{content:'';position:absolute;top:3px;left:3px;width:22px;height:22px;border-radius:50%;background:var(--app-surface);box-shadow:0 1px 3px #0002;transition:transform .15s}
.source-field-switch input:checked+span{background:var(--app-accent)}
.source-field-switch input:checked+span::after{transform:translateX(20px)}
.source-field-switch input:focus-visible+span,.source-field input:focus-visible,.source-field select:focus-visible,.source-field-preview:focus-visible,.source-field-tabs>button:focus-visible{outline:2px solid var(--app-accent);outline-offset:3px}
.source-field-switch input:disabled+span,.source-field :disabled{opacity:.55;cursor:default}
.source-field-error{padding:12px;background:var(--app-danger-soft);color:var(--app-danger);border-radius:8px;font-size:var(--ui-font-body)}
.source-field-overlay{position:fixed;z-index:120;inset:0;display:flex;flex-direction:column;padding:var(--safe-top) 0 var(--safe-bottom);background:var(--app-panel-soft)}
.source-field-overlay>header{display:flex;align-items:center;justify-content:space-between;gap:12px;min-height:62px;padding:0 20px;border-bottom:1px solid var(--app-line);background:var(--app-surface)}
.source-field-overlay>header>div{display:grid;min-width:0;gap:3px;text-align:center}
.source-field-overlay strong{font-size:var(--ui-font-body);color:var(--app-text)}
.source-field-overlay small{font-size:var(--ui-font-caption);color:var(--app-subtle);overflow:hidden;text-overflow:ellipsis}
.source-field-overlay button{min-height:44px;padding:0 10px;border:0;background:transparent;color:var(--app-accent);font-size:var(--ui-font-body);cursor:pointer}
.source-field-overlay textarea{flex:1;min-height:0;resize:none;margin:16px;padding:16px;border:1px solid var(--app-line);border-radius:8px;background:var(--app-surface);color:var(--app-text);font:var(--ui-font-code)/1.7 ui-monospace,SFMono-Regular,Consolas,monospace;tab-size:2;outline-offset:2px}
@media(min-width:1100px){.source-field-overlay{inset:5vh 12vw;border:1px solid var(--app-line);border-radius:12px;box-shadow:0 0 0 100vmax #18203466;overflow:hidden}}
@media(max-width:700px){.source-field-tabs>button{padding:0 15px}.source-field-list{gap:19px 14px;padding:22px 0}.source-field-label{font-size:var(--ui-font-body);gap:5px}.source-field-overlay>header{padding:0 10px}.source-field-overlay textarea{margin:10px;padding:12px}}
@media(prefers-reduced-motion:reduce){.source-field-switch>span,.source-field-switch>span::after{transition:none}}
</style>

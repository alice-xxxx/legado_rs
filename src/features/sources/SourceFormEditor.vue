<script setup lang="ts">
import { Popup as KPopup, ListInput as KListInput, Toggle as KToggle, Button as KButton } from "konsta/vue";
import SubpageHeader from "../../ui/SubpageHeader.vue";
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
      <KButton rounded :tonal="currentTab.id === tab.id" :clear="!(currentTab.id === tab.id)" inline class="gap-2" v-for="tab in tabs" :key="tab.id" type="button" role="tab"
        :aria-selected="currentTab.id === tab.id" :class="{ active: currentTab.id === tab.id }"
        @click="activeTab = tab.id">{{ tab.label }}</KButton>
    </nav>
    <div class="source-field-list" role="tabpanel" :aria-label="currentTab.label">
      <div v-for="field in currentTab.fields" :key="field.key" class="source-field"
        :class="{ 'source-field--compact': ['toggle', 'type', 'number'].includes(field.kind) }">
        <label v-if="field.kind !== 'toggle'" :for="`source-field-${field.key}`" class="source-field-label">
          <span>{{ field.label }}</span><small>{{ field.key.split('.').slice(-1)[0] }}</small>
        </label>
        <KButton rounded inline clear v-if="field.kind === 'long'" :id="`source-field-${field.key}`" type="button"
          class="source-field-preview h-auto rounded-xl bg-[var(--app-surface)] text-current text-start justify-between gap-2" :class="{ empty: !draft[field.key] }" :disabled="disabled" @click="openField(field)">
          <span>{{ preview(field) }}</span><PrototypeIcon name="right" />
        </KButton>
        <label v-else-if="field.kind === 'toggle'" class="source-field-switch">
          <span class="source-field-label">{{ field.label }} <small>{{ field.key.split('.').slice(-1)[0] }}</small></span>
          <KToggle component="span" :checked="draft[field.key] === 'true'" :disabled="disabled"
            @change="draft[field.key] = ($event.target as HTMLInputElement).checked ? 'true' : 'false'"  />
        </label>
        <KListInput component="div" v-else-if="field.kind === 'type'" :input-id="`source-field-${field.key}`" :disabled="disabled" type="select" :value="draft[field.key]" @change="draft[field.key] = ($event.target as HTMLSelectElement).value">
          <option v-for="option in bookSourceTypeOptions(draft[field.key] ?? '0')" :key="option.value" :value="option.value">{{ option.label }}</option>
        </KListInput>
        <KListInput component="div" v-else :input-id="`source-field-${field.key}`" :disabled="disabled" :inputmode="field.kind === 'number' ? 'numeric' : 'text'" type="text" spellcheck="false" autocomplete="off" :value="draft[field.key]" @input="draft[field.key] = ($event.target as HTMLInputElement).value" />
      </div>
    </div>
    <p v-if="error" class="source-field-error" role="alert">{{ error }}</p>
    <Teleport to="body">
      <KPopup v-if="editingField" opened class="source-field-overlay z-[120]" @backdropclick="closeField(true)">
        <section class="flex h-full min-h-0 flex-col" role="dialog" aria-modal="true" :aria-label="editingField.label" data-back-layer
          @keydown="trapFieldFocus" @keydown.esc.stop.prevent="closeField(true)">
        <SubpageHeader :title="editingField.label" :subtitle="editingField.key" back-label="取消编辑" local-back :back-disabled="disabled" @back="closeField(true)">
          <template #actions>
          <KButton rounded inline clear class="gap-2" type="button" :disabled="disabled" @click="closeField()">完成</KButton>
          </template>
        </SubpageHeader>
        <label class="sr-only" for="source-long-value">{{ editingField.label }}</label>
        <KListInput component="div" input-id="source-long-value" class="source-code-field" :disabled="disabled" spellcheck="false" autocapitalize="off" autocomplete="off" type="textarea" input-class="h-full! resize-none font-mono leading-7" :value="draft[editingField.key]" @input="draft[editingField.key] = ($event.target as HTMLTextAreaElement).value" />
        </section>
      </KPopup>
    </Teleport>
  </div>
</template>

<style scoped>
@layer components {
.source-field-editor{min-width:0;color:var(--app-text,var(--app-text))}
.source-field-tabs{position:sticky;top:0;z-index:5;display:flex;overflow-x:auto;background:var(--app-background);border-bottom:1px solid var(--app-line);scrollbar-width:none}
.source-field-tabs>button{flex:1 0 auto;min-height:48px;padding:0 18px}
.source-field-tabs>button.active{border-bottom-color:var(--app-accent);color:var(--app-accent);font-weight:650}
.source-field-list{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:20px 22px;padding:24px 0}
.source-field{display:grid;align-content:start;gap:8px;min-width:0;grid-column:1/-1}
.source-field--compact{grid-column:auto}
.source-field-label{display:flex;align-items:baseline;gap:8px;flex-wrap:wrap;font-size:var(--ui-font-body);line-height:1.4;color:var(--app-muted)}
.source-field-label small{font-size:var(--ui-font-caption);font-weight:400;color:var(--app-subtle);overflow-wrap:anywhere}
.source-field .k-list-input{box-sizing:border-box;width:100%;min-width:0}
.source-field-preview{box-sizing:border-box;width:100%;min-width:0;min-height:46px;padding:11px 13px;line-height:1.5}
.source-field-preview{display:flex;align-items:center;gap:12px;text-align:left}
.source-field-preview>span{flex:1;min-width:0;overflow:hidden;white-space:nowrap;text-overflow:ellipsis;font:var(--ui-font-code)/1.6 ui-monospace,SFMono-Regular,Consolas,monospace}
.source-field-preview.empty>span{color:var(--app-subtle);font-family:inherit}
.source-field-preview :deep(svg){flex:none;width:18px;height:18px;color:var(--app-subtle)}
.source-field-switch{display:flex;align-items:center;justify-content:space-between;gap:12px;min-height:52px;cursor:pointer}
.source-field-preview:focus-visible, .source-field-tabs>button:focus-visible{outline:2px solid var(--app-accent);outline-offset:3px}
.source-field :disabled{opacity:.55;cursor:default}
.source-field-error{padding:12px;background:var(--app-danger-soft);color:var(--app-danger);border-radius:8px;font-size:var(--ui-font-body)}
@media(max-width:700px){.source-field-tabs>button{padding:0 15px}.source-field-list{gap:19px 14px;padding:22px 0}.source-field-label{font-size:var(--ui-font-body);gap:5px}}
}

@layer components {
.source-code-field { flex: 1; min-height: 0; padding: 16px; overflow: hidden; }
.source-code-field :deep(> div), .source-code-field :deep(> div > div), .source-code-field :deep(> div > div > div) { height: 100%; }
.source-code-field :deep(textarea) { min-height: 100%; }
}
</style>

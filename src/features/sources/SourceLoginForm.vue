<script setup lang="ts">
import { ListInput as KListInput, Toggle as KToggle, Button as KButton } from "konsta/vue";
import { computed, ref, useId, watch } from "vue";
import type { SourceLoginAction, SourceLoginField, SourceLoginForm, SourceLoginLayoutRow } from "../../api/types";

const props = defineProps<{
  form: Extract<SourceLoginForm, { mode: "form" }>;
  modelValue: Record<string, string>;
  disabled: boolean;
  busy: boolean;
}>();
const emit = defineEmits<{
  "update:modelValue": [value: Record<string, string>];
  execute: [actionId?: number];
}>();
const instanceId = useId();
const visiblePasswords = ref<string[]>([]);
let currentValues = props.modelValue;
watch(() => props.modelValue, value => { currentValues = value; }, { flush: "sync" });
type DisplayRow = SourceLoginLayoutRow & { field?: SourceLoginField; action?: SourceLoginAction };
const rows = computed<DisplayRow[]>(() => {
  const fields = new Map(props.form.fields.map(field => [field.name, field]));
  const actions = new Map(props.form.actions.map(action => [action.id, action]));
  const layout: SourceLoginLayoutRow[] = props.form.layout ?? [
    ...props.form.fields.map(field => ({ kind: "field" as const, name: field.name, cols: 1, rows: 1 })),
    ...props.form.actions.map(action => ({ kind: "action" as const, id: action.id, cols: 2, rows: 1 })),
  ];
  return layout.map(row => ({
    ...row,
    field: row.kind === "field" ? fields.get(row.name) : undefined,
    action: row.kind === "action" ? actions.get(row.id) : undefined,
  }));
});
function rowStyle(row: SourceLoginLayoutRow) {
  const cols = Math.max(1, Math.min(4, row.cols));
  const height = Math.max(1, Math.min(12, row.rows));
  return { gridColumn: `span ${12 / cols}`, gridRow: `span ${height}` };
}
function fieldId(name: string, index: number): string { return `${instanceId}-${index}-${name}`; }
function change(name: string, value: string, actionId?: number): void {
  if (props.disabled) return;
  const next = Object.assign(Object.create(null), currentValues) as Record<string, string>;
  next[name] = value;
  currentValues = next;
  emit("update:modelValue", next);
  if (actionId !== undefined) execute(actionId);
}
function togglePassword(name: string): void {
  visiblePasswords.value = visiblePasswords.value.includes(name)
    ? visiblePasswords.value.filter(value => value !== name) : [...visiblePasswords.value, name];
}
function execute(actionId?: number): void {
  if (!props.disabled && (actionId !== undefined || props.form.canLogin)) emit("execute", actionId);
}
</script>

<template>
  <form class="source-login-fields" :aria-busy="busy" @submit.prevent="execute()">
    <div class="source-login-grid">
      <div v-for="(row, index) in rows" :key="index" class="source-login-row" :style="rowStyle(row)"
        :class="{ 'source-login-row--title': row.kind === 'title', 'source-login-row--action': row.kind === 'action', 'source-login-row--tall': row.rows > 1 }">
        <p v-if="row.kind === 'title'" class="source-login-section-title">{{ row.label }}</p>
        <KButton rounded inline clear v-else-if="row.action" type="button" class="source-login-rule-button gap-2" :disabled="disabled"
          @click="execute(row.action.id)">{{ row.action.label }}</KButton>
        <template v-else-if="row.field">
          <label v-if="row.field.type === 'toggle'" class="source-login-toggle" :class="{ 'is-disabled': disabled }">
            <span>{{ row.field.label }}</span>
            <KToggle component="span" :disabled="disabled"
              :checked="modelValue[row.field.name] === (row.field.choices?.[1] ?? 'true')"
              @change="change(row.field.name, ($event.target as HTMLInputElement).checked ? (row.field.choices?.[1] ?? 'true') : (row.field.choices?.[0] ?? 'false'), row.kind === 'field' ? row.id : undefined)"  />
          </label>
          <template v-else>
            <label class="source-login-label" :for="fieldId(row.field.name, index)">{{ row.field.label }}</label>
            <KListInput component="div" v-if="row.field.type === 'select'" :input-id="fieldId(row.field.name, index)" :value="modelValue[row.field.name] ?? ''" :disabled="disabled" type="select" @change="change(row.field.name, ($event.target as HTMLSelectElement).value, row.kind === 'field' ? row.id : undefined)">
              <option v-for="choice in row.field.choices ?? []" :key="choice" :value="choice">{{ choice }}</option>
            </KListInput>
            <div v-else-if="row.field.password || row.field.type === 'password'" class="source-login-password">
              <KListInput component="div" :input-id="fieldId(row.field.name, index)" :value="modelValue[row.field.name] ?? ''" :type="visiblePasswords.includes(row.field.name) ? 'text' : 'password'" :disabled="disabled" autocomplete="new-password" maxlength="4096" @change="execute(row.kind === 'field' ? row.id : undefined)" @input="change(row.field.name, ($event.target as HTMLInputElement).value)" />
              <KButton rounded inline clear class="gap-2" type="button" :aria-label="visiblePasswords.includes(row.field.name) ? '隐藏密码' : '显示密码'"
                :aria-pressed="visiblePasswords.includes(row.field.name)" :disabled="disabled" @click="togglePassword(row.field.name)">
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true">
                  <path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12Z"/><circle cx="12" cy="12" r="3"/>
                  <path v-if="visiblePasswords.includes(row.field.name)" d="m4 4 16 16"/>
                </svg>
              </KButton>
            </div>
            <KListInput component="div" v-else-if="row.rows > 1" :input-id="fieldId(row.field.name, index)" :value="modelValue[row.field.name] ?? ''" :disabled="disabled" maxlength="4096" spellcheck="false" autocomplete="off" @change="execute(row.kind === 'field' ? row.id : undefined)" type="textarea" input-class="h-auto! min-h-28 resize-y" :input-style="{ minHeight: '96px' }" @input="change(row.field.name, ($event.target as HTMLTextAreaElement).value)" />
            <KListInput component="div" v-else :input-id="fieldId(row.field.name, index)" :value="modelValue[row.field.name] ?? ''" type="text" :disabled="disabled" maxlength="4096" spellcheck="false" autocomplete="off" autocapitalize="off" @change="execute(row.kind === 'field' ? row.id : undefined)" @input="change(row.field.name, ($event.target as HTMLInputElement).value)" />
          </template>
        </template>
      </div>
    </div>
    <p v-if="form.fields.some(field => field.password || field.type === 'password')" class="source-login-password-note">
      密码留空时沿用已保存的密码。
    </p>
    <KButton rounded inline v-if="form.canLogin" type="submit" class="source-login-submit gap-2" :disabled="disabled">{{ busy ? '正在处理…' : '登录' }}</KButton>
  </form>
</template>

<style scoped>
@layer components {
.source-login-fields{display:grid;gap:12px;min-width:0}
.source-login-grid{display:grid;grid-template-columns:repeat(12,minmax(0,1fr));grid-auto-rows:minmax(var(--ui-control-height),auto);gap:12px;align-items:stretch}
.source-login-row{display:flex;flex-direction:column;justify-content:flex-end;gap:6px;min-width:0;overflow-wrap:anywhere}
.source-login-row--title{justify-content:center;padding-top:4px}
.source-login-section-title{margin:0;color:var(--app-text);font-size:var(--ui-font-body);font-weight:650;line-height:1.6;white-space:pre-wrap}
.source-login-label{color:var(--app-muted);font-size:var(--ui-font-body);line-height:1.4}
.source-login-row .k-list-input{box-sizing:border-box;min-width:0;width:100%}
.source-login-row .k-list-input{flex:1}
.source-login-fields button:focus-visible{outline:2px solid var(--app-accent);outline-offset:2px}
.source-login-password{display:flex;align-items:center;min-width:0;border:1px solid var(--app-line);border-radius:var(--ui-radius);background:var(--app-surface)}
.source-login-password .k-list-input{flex:1}
.source-login-password>button{display:grid;place-items:center;flex:none;align-self:stretch;width:42px}
.source-login-password svg{width:19px;height:19px}
.source-login-toggle{position:relative;display:flex;align-items:center;justify-content:space-between;gap:10px;min-height:46px;padding:10px 12px;border:1px solid var(--app-line);border-radius:8px;background:var(--app-panel-soft);font-size:var(--ui-font-body);cursor:pointer}
.source-login-toggle.is-disabled{opacity:.55;cursor:default}
.source-login-rule-button, .source-login-submit{min-height:var(--ui-control-height);padding:10px 12px;line-height:1.4;overflow-wrap:anywhere}
.source-login-row--tall .source-login-rule-button, .source-login-row--tall .source-login-toggle{flex:1}
.source-login-submit{justify-self:end;padding-inline:16px}
.source-login-password-note{margin:0;color:var(--app-muted);font-size:var(--ui-font-caption);line-height:1.5}
.source-login-fields :disabled{opacity:.55;cursor:default}
}
</style>

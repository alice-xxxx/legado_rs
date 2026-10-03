<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import {
  deleteTxtTocRule,
  loadTxtTocRules,
  upsertTxtTocRule,
  type TxtTocRule,
} from "../api/txtToc";
import type { ResourceDescriptor } from "../api/app";

interface RuleDraft {
  name: string;
  rule: string;
  example: string;
  enable: boolean;
}

const props = withDefaults(defineProps<{ busy?: boolean; resource?: ResourceDescriptor }>(), { busy: false });
const emit = defineEmits<{
  close: [];
  updated: [resource: ResourceDescriptor];
}>();

const rules = ref<TxtTocRule[]>([]);
const loading = ref(true);
const saving = ref(false);
const loadError = ref("");
const errorMessage = ref("");
const successMessage = ref("");
const editingId = ref<string | null>(null);
const editingSerialNumber = ref(0);
const deleteTarget = ref<TxtTocRule | null>(null);
const draft = ref<RuleDraft>(emptyDraft());

const orderedRules = computed(() => [...rules.value].sort((left, right) =>
  left.serialNumber - right.serialNumber || left.id.localeCompare(right.id),
));
const busy = computed(() => props.busy || loading.value || saving.value);
const atRuleLimit = computed(() => editingId.value === null && rules.value.length >= 64);
const canSave = computed(() =>
  !busy.value && !atRuleLimit.value && draft.value.name.trim().length > 0 && draft.value.rule.trim().length > 0,
);

onMounted(() => void reload(props.resource));
watch(() => props.resource, (resource) => {
  if (resource && !saving.value) void reload(resource);
}, { deep: true, flush: "sync" });

function emptyDraft(): RuleDraft {
  return { name: "", rule: "", example: "", enable: true };
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function resetEditor(): void {
  editingId.value = null;
  editingSerialNumber.value = orderedRules.value.length;
  draft.value = emptyDraft();
  errorMessage.value = "";
  successMessage.value = "";
}

async function reload(resource?: ResourceDescriptor): Promise<void> {
  loading.value = true;
  loadError.value = "";
  try {
    const loaded = await loadTxtTocRules(resource);
    rules.value = loaded.document.rules;
  } catch (error) {
    loadError.value = errorText(error);
  } finally {
    loading.value = false;
  }
}

function editRule(rule: TxtTocRule): void {
  if (busy.value) return;
  editingId.value = rule.id;
  editingSerialNumber.value = rule.serialNumber;
  draft.value = {
    name: rule.name,
    rule: rule.rule,
    example: rule.example ?? "",
    enable: rule.enable,
  };
  errorMessage.value = "";
  successMessage.value = "";
}

function createRuleId(): string {
  const randomPart = globalThis.crypto?.randomUUID?.().replace(/-/g, "")
    ?? Math.random().toString(36).slice(2, 14);
  return `txt-toc-${Date.now().toString(36)}-${randomPart}`;
}

async function saveRule(): Promise<void> {
  errorMessage.value = "";
  successMessage.value = "";
  if (!draft.value.name.trim()) {
    errorMessage.value = "请填写规则名称。";
    return;
  }
  if (!draft.value.rule.trim()) {
    errorMessage.value = "正则表达式不能为空。";
    return;
  }

  saving.value = true;
  try {
    const rule: TxtTocRule = {
      id: editingId.value ?? createRuleId(),
      name: draft.value.name.trim(),
      rule: draft.value.rule,
      ...(draft.value.example.trim() ? { example: draft.value.example } : {}),
      serialNumber: editingId.value ? editingSerialNumber.value : orderedRules.value.length,
      enable: draft.value.enable,
    };
    const wasEditing = editingId.value !== null;
    const resource = await upsertTxtTocRule(rule);
    await reloadAfterMutation(resource);
    resetEditor();
    successMessage.value = wasEditing ? "规则已保存并从资源重新读取。" : "规则已添加并从资源重新读取。";
  } catch (error) {
    errorMessage.value = errorText(error);
    // Rust owns regex parsing and size limits. Re-read after any command error
    // so the list always reflects the persisted document, including partial
    // multi-rule reorder operations.
    await reload();
  } finally {
    saving.value = false;
  }
}

async function persistRule(rule: TxtTocRule, notice: string): Promise<void> {
  errorMessage.value = "";
  successMessage.value = "";
  saving.value = true;
  try {
    const resource = await upsertTxtTocRule(rule);
    await reloadAfterMutation(resource);
    successMessage.value = notice;
  } catch (error) {
    errorMessage.value = errorText(error);
    await reload();
  } finally {
    saving.value = false;
  }
}

async function toggleRule(rule: TxtTocRule, enable: boolean): Promise<void> {
  if (busy.value) return;
  await persistRule({ ...rule, enable }, enable ? "规则已启用。" : "规则已停用。");
}

async function moveRule(ruleId: string, direction: -1 | 1): Promise<void> {
  if (busy.value) return;
  const reordered = orderedRules.value;
  const index = reordered.findIndex((rule) => rule.id === ruleId);
  const targetIndex = index + direction;
  if (index < 0 || targetIndex < 0 || targetIndex >= reordered.length) return;
  [reordered[index], reordered[targetIndex]] = [reordered[targetIndex], reordered[index]];

  saving.value = true;
  errorMessage.value = "";
  successMessage.value = "";
  try {
    let resource: ResourceDescriptor | undefined;
    for (let serialNumber = 0; serialNumber < reordered.length; serialNumber += 1) {
      const rule = reordered[serialNumber];
      if (rule.serialNumber === serialNumber) continue;
      resource = await upsertTxtTocRule({ ...rule, serialNumber });
    }
    if (resource) await reloadAfterMutation(resource);
    else await reload();
    successMessage.value = "规则顺序已保存。";
  } catch (error) {
    errorMessage.value = errorText(error);
    await reload();
  } finally {
    saving.value = false;
  }
}

function requestDelete(rule: TxtTocRule): void {
  if (busy.value) return;
  deleteTarget.value = rule;
  errorMessage.value = "";
}

async function confirmDelete(): Promise<void> {
  const rule = deleteTarget.value;
  if (!rule || busy.value) return;
  saving.value = true;
  errorMessage.value = "";
  successMessage.value = "";
  try {
    const resource = await deleteTxtTocRule(rule.id);
    deleteTarget.value = null;
    if (editingId.value === rule.id) resetEditor();
    await reloadAfterMutation(resource);
    successMessage.value = `“${rule.name}”已删除。`;
  } catch (error) {
    errorMessage.value = errorText(error);
    await reload();
  } finally {
    saving.value = false;
  }
}

async function reloadAfterMutation(resource: ResourceDescriptor): Promise<void> {
  const loaded = await loadTxtTocRules();
  rules.value = loaded.document.rules;
  emit("updated", resource);
}
</script>

<template>
  <section
    class="txt-toc-editor"
    role="dialog"
    aria-modal="true"
    aria-labelledby="txt-toc-editor-title"
    data-testid="txt-toc-rules-editor"
  >
    <header class="txt-toc-editor__header">
      <div>
        <p class="txt-toc-editor__eyebrow">本地导入设置</p>
        <h2 id="txt-toc-editor-title">TXT 章节识别规则</h2>
        <p>这些规则用于本地 TXT 目录识别；新增或修改后只影响之后导入的 TXT。</p>
      </div>
      <button type="button" class="txt-toc-editor__icon-button" aria-label="关闭 TXT 规则设置" data-testid="txt-toc-close" @click="emit('close')">×</button>
    </header>

    <div class="txt-toc-editor__body">
      <div v-if="loading" class="txt-toc-editor__message" role="status" data-testid="txt-toc-loading">正在读取规则…</div>
      <div v-else-if="loadError" class="txt-toc-editor__error" role="alert" data-testid="txt-toc-load-error">
        <strong>无法读取规则</strong><span>{{ loadError }}</span>
        <button type="button" class="txt-toc-editor__button txt-toc-editor__button--secondary" :disabled="busy" @click="reload()">重试</button>
      </div>

      <template v-else>
        <p class="txt-toc-editor__notice">最多保存 64 条规则。表达式会在保存时检查格式，错误会在此处显示。</p>

        <form class="txt-toc-editor__form" data-testid="txt-toc-rule-form" @submit.prevent="saveRule">
          <div class="txt-toc-editor__form-heading">
            <div><h3>{{ editingId ? "编辑规则" : "添加规则" }}</h3><p>规则名称和表达式必填。</p></div>
            <span v-if="editingId" class="txt-toc-editor__editing-pill">正在编辑</span>
          </div>

          <label class="txt-toc-editor__field">
            <span>规则名称</span>
            <input v-model="draft.name" type="text" maxlength="256" autocomplete="off" :disabled="busy" data-testid="txt-toc-rule-name" placeholder="例如：常见章节标题" />
          </label>

          <label class="txt-toc-editor__field">
            <span>章节标题表达式</span>
            <textarea v-model="draft.rule" rows="4" spellcheck="false" autocomplete="off" :disabled="busy" data-testid="txt-toc-rule-pattern" placeholder="例如：^第[一二三四五六七八九十百千0-9]+章.*$"></textarea>
            <small>留空或表达式不正确时无法保存。</small>
          </label>

          <label class="txt-toc-editor__field">
            <span>示例章节标题（可选）</span>
            <input v-model="draft.example" type="text" maxlength="2048" autocomplete="off" :disabled="busy" data-testid="txt-toc-rule-example" placeholder="填写一个用于说明此规则的标题" />
            <small>示例仅用于说明这条规则预期匹配的章节标题。</small>
          </label>

          <label class="txt-toc-editor__checkbox">
            <input v-model="draft.enable" type="checkbox" :disabled="busy" data-testid="txt-toc-rule-enabled" />
            <span>启用此规则</span>
          </label>
          <p v-if="atRuleLimit" class="txt-toc-editor__limit" role="status">已达到 64 条规则上限；删除一条后可以继续添加。</p>

          <div v-if="errorMessage" class="txt-toc-editor__error" role="alert" data-testid="txt-toc-error">{{ errorMessage }}</div>
          <div v-if="successMessage" class="txt-toc-editor__success" role="status" data-testid="txt-toc-success">{{ successMessage }}</div>

          <div class="txt-toc-editor__form-actions">
            <button v-if="editingId" type="button" class="txt-toc-editor__button txt-toc-editor__button--secondary" :disabled="busy" @click="resetEditor">取消编辑</button>
            <button type="submit" class="txt-toc-editor__button txt-toc-editor__button--primary" data-testid="txt-toc-rule-save" :disabled="!canSave">
              {{ saving ? "正在保存…" : editingId ? "保存修改" : "添加规则" }}
            </button>
          </div>
        </form>

        <section class="txt-toc-editor__list" aria-label="已保存的 TXT 规则">
          <header class="txt-toc-editor__list-heading">
            <div><h3>已保存规则</h3><p>排在前面的规则优先参与目录识别。</p></div>
            <span class="txt-toc-editor__count">{{ rules.length }} / 64</span>
          </header>

          <div v-if="!orderedRules.length" class="txt-toc-editor__empty" data-testid="txt-toc-rules-empty">
            还没有自定义规则。添加后，导入 TXT 时会使用已启用的规则识别章节。
          </div>

          <article v-for="(rule, index) in orderedRules" :key="rule.id" class="txt-toc-editor__rule" :data-testid="`txt-toc-rule-${rule.id}`">
            <div class="txt-toc-editor__rule-order" :aria-label="`优先级 ${index + 1}`">{{ index + 1 }}</div>
            <div class="txt-toc-editor__rule-main">
              <div class="txt-toc-editor__rule-title">
                <strong>{{ rule.name }}</strong>
                <span class="txt-toc-editor__state" :class="rule.enable ? 'is-enabled' : 'is-disabled'">{{ rule.enable ? "已启用" : "已停用" }}</span>
              </div>
              <code>{{ rule.rule }}</code>
              <p v-if="rule.example" class="txt-toc-editor__example">示例：{{ rule.example }}</p>
            </div>
            <div class="txt-toc-editor__rule-actions">
              <label class="txt-toc-editor__mini-toggle" :aria-label="`${rule.name}启用状态`">
                <input type="checkbox" :checked="rule.enable" :disabled="busy" :data-testid="`txt-toc-rule-toggle-${rule.id}`" @change="toggleRule(rule, ($event.target as HTMLInputElement).checked)" />
                <span>启用</span>
              </label>
              <button type="button" class="txt-toc-editor__icon-button" :disabled="busy || index === 0" :aria-label="`${rule.name}上移`" :data-testid="`txt-toc-rule-up-${rule.id}`" @click="moveRule(rule.id, -1)">↑</button>
              <button type="button" class="txt-toc-editor__icon-button" :disabled="busy || index === orderedRules.length - 1" :aria-label="`${rule.name}下移`" :data-testid="`txt-toc-rule-down-${rule.id}`" @click="moveRule(rule.id, 1)">↓</button>
              <button type="button" class="txt-toc-editor__icon-button" :disabled="busy" :aria-label="`编辑${rule.name}`" :data-testid="`txt-toc-rule-edit-${rule.id}`" @click="editRule(rule)">✎</button>
              <button type="button" class="txt-toc-editor__icon-button txt-toc-editor__icon-button--danger" :disabled="busy" :aria-label="`删除${rule.name}`" :data-testid="`txt-toc-rule-delete-${rule.id}`" @click="requestDelete(rule)">×</button>
            </div>
          </article>
        </section>
      </template>
    </div>

    <div v-if="deleteTarget" class="txt-toc-editor__scrim" data-testid="txt-toc-delete-dialog">
      <section class="txt-toc-editor__confirm" role="alertdialog" aria-modal="true" aria-labelledby="txt-toc-delete-title" aria-describedby="txt-toc-delete-description">
        <h3 id="txt-toc-delete-title">删除这条规则？</h3>
        <p id="txt-toc-delete-description">“{{ deleteTarget.name }}”将从本地 TXT 目录识别设置中移除。</p>
        <div v-if="errorMessage" class="txt-toc-editor__error" role="alert">{{ errorMessage }}</div>
        <div class="txt-toc-editor__confirm-actions">
          <button type="button" class="txt-toc-editor__button txt-toc-editor__button--secondary" :disabled="busy" data-testid="txt-toc-delete-cancel" @click="deleteTarget = null">取消</button>
          <button type="button" class="txt-toc-editor__button txt-toc-editor__button--danger" :disabled="busy" data-testid="txt-toc-delete-confirm" @click="confirmDelete">{{ saving ? "正在删除…" : "删除规则" }}</button>
        </div>
      </section>
    </div>

    <footer class="txt-toc-editor__footer">
      <span>本地 TXT 设置 · {{ rules.length }} 条规则</span>
      <button type="button" class="txt-toc-editor__button txt-toc-editor__button--secondary" data-testid="txt-toc-close-footer" @click="emit('close')">完成</button>
    </footer>
  </section>
</template>

<style scoped>
.txt-toc-editor { position:relative; display:flex; width:min(880px,calc(100vw - 32px)); max-height:min(92vh,900px); flex-direction:column; overflow:hidden; border:1px solid #e1e8e1; border-radius:16px; background:#fff; box-shadow:0 22px 70px rgb(37 57 43 / 22%); color:#405047; }
.txt-toc-editor__header { display:flex; flex:none; align-items:flex-start; justify-content:space-between; gap:18px; padding:24px 26px 19px; border-bottom:1px solid #edf0ed; background:#fbfcfa; }
.txt-toc-editor__header h2 { margin:0; color:#35443a; font-size:21px; line-height:1.35; }
.txt-toc-editor__header p:not(.txt-toc-editor__eyebrow) { max-width:610px; margin:8px 0 0; color:#7b887e; font-size:14px; line-height:1.6; }
.txt-toc-editor__eyebrow { margin:0 0 5px; color:#8da093; font-size:12px; font-weight:800; letter-spacing:.13em; }
.txt-toc-editor__icon-button { display:inline-grid; width:42px; min-width:42px; height:42px; place-items:center; border:1px solid #e3e9e3; border-radius:8px; color:#66766a; background:#fff; font-size:18px; cursor:pointer; }
.txt-toc-editor__icon-button:hover:not(:disabled) { border-color:#b9cbbd; color:#3c725f; background:#f3f7f3; }
.txt-toc-editor__icon-button:disabled { opacity:.45; cursor:not-allowed; }
.txt-toc-editor__icon-button--danger { color:#966358; }
.txt-toc-editor__body { min-height:0; flex:1; overflow:auto; padding:20px 26px 25px; }
.txt-toc-editor__notice { margin:0 0 16px; padding:11px 13px; border:1px solid #e7eee6; border-radius:8px; color:#687a6c; background:#f7faf6; font-size:13px; line-height:1.55; }
.txt-toc-editor__form { display:grid; gap:13px; padding:17px; border:1px solid #e4ebe4; border-radius:12px; background:#fafcf9; }
.txt-toc-editor__form-heading,.txt-toc-editor__list-heading { display:flex; align-items:center; justify-content:space-between; gap:12px; }
.txt-toc-editor__form-heading h3,.txt-toc-editor__list-heading h3 { margin:0; color:#485a4d; font-size:16px; }
.txt-toc-editor__form-heading p,.txt-toc-editor__list-heading p { margin:4px 0 0; color:#7b887e; font-size:13px; line-height:1.5; }
.txt-toc-editor__editing-pill,.txt-toc-editor__count { flex:none; padding:5px 9px; border-radius:999px; color:#58765f; background:#edf4ec; font-size:12px; font-weight:700; }
.txt-toc-editor__field { display:grid; gap:6px; color:#526257; font-size:14px; font-weight:700; }
.txt-toc-editor__field input,.txt-toc-editor__field textarea { width:100%; min-width:0; box-sizing:border-box; padding:10px 12px; border:1px solid #dce5dc; border-radius:8px; outline:none; color:#36473b; background:#fff; font-family:inherit; font-size:14px; font-weight:400; line-height:1.55; }
.txt-toc-editor__field input { min-height:44px; }
.txt-toc-editor__field textarea { min-height:96px; resize:vertical; font-family:ui-monospace,SFMono-Regular,Consolas,monospace; }
.txt-toc-editor__field input:focus,.txt-toc-editor__field textarea:focus { border-color:#80a58a; box-shadow:0 0 0 3px rgb(91 139 102 / 12%); }
.txt-toc-editor__field input:disabled,.txt-toc-editor__field textarea:disabled { color:#87918a; background:#f4f6f3; }
.txt-toc-editor__field small { color:#7d8a80; font-size:12px; font-weight:400; line-height:1.5; }
.txt-toc-editor__checkbox,.txt-toc-editor__mini-toggle { display:inline-flex; align-items:center; gap:8px; color:#5c6b60; font-size:14px; cursor:pointer; }
.txt-toc-editor__limit { margin:0; color:#806b42; font-size:12px; line-height:1.5; }
.txt-toc-editor__checkbox input,.txt-toc-editor__mini-toggle input { width:18px; height:18px; accent-color:#3c725f; }
.txt-toc-editor__form-actions { display:flex; justify-content:flex-end; gap:9px; }
.txt-toc-editor__button { min-height:44px; padding:0 14px; border:1px solid transparent; border-radius:8px; font-size:14px; font-weight:700; cursor:pointer; }
.txt-toc-editor__button:disabled { opacity:.5; cursor:not-allowed; }
.txt-toc-editor__button--primary { color:#fff; background:#3c725f; }
.txt-toc-editor__button--primary:hover:not(:disabled) { background:#315f4e; }
.txt-toc-editor__button--secondary { border-color:#dfe7df; color:#5d6f62; background:#fff; }
.txt-toc-editor__button--secondary:hover:not(:disabled) { background:#f5f8f4; }
.txt-toc-editor__button--danger { color:#fff; background:#9b5e51; }
.txt-toc-editor__button--danger:hover:not(:disabled) { background:#844c41; }
.txt-toc-editor__error,.txt-toc-editor__success,.txt-toc-editor__message { display:flex; flex-wrap:wrap; align-items:center; gap:8px; padding:11px 12px; border-radius:8px; font-size:13px; line-height:1.5; }
.txt-toc-editor__error { border:1px solid #efd5cf; color:#8a4d43; background:#fff7f5; }
.txt-toc-editor__success { border:1px solid #dcebdd; color:#54765a; background:#f3faf2; }
.txt-toc-editor__message { color:#78867b; background:#f7f9f6; }
.txt-toc-editor__list { margin-top:23px; }
.txt-toc-editor__list-heading { margin-bottom:11px; }
.txt-toc-editor__empty { display:grid; min-height:108px; place-items:center; padding:18px; border:1px dashed #dfe7df; border-radius:10px; color:#7c897f; font-size:14px; line-height:1.6; text-align:center; }
.txt-toc-editor__rule { display:grid; grid-template-columns:34px minmax(0,1fr) auto; align-items:center; gap:13px; padding:13px 12px; border:1px solid #e6ece6; border-radius:10px; background:#fff; }
.txt-toc-editor__rule + .txt-toc-editor__rule { margin-top:8px; }
.txt-toc-editor__rule-order { display:grid; width:30px; height:30px; place-items:center; border-radius:50%; color:#617668; background:#f0f5ef; font-size:13px; font-weight:800; }
.txt-toc-editor__rule-main { min-width:0; display:grid; gap:6px; }
.txt-toc-editor__rule-title { display:flex; flex-wrap:wrap; align-items:center; gap:8px; }
.txt-toc-editor__rule-title strong { color:#46574b; font-size:14px; }
.txt-toc-editor__state { padding:3px 7px; border-radius:999px; font-size:11px; font-weight:700; }
.txt-toc-editor__state.is-enabled { color:#507458; background:#edf5ed; }
.txt-toc-editor__state.is-disabled { color:#7d817c; background:#f1f2f0; }
.txt-toc-editor__rule code { display:block; overflow-wrap:anywhere; color:#526459; font:12px/1.5 ui-monospace,SFMono-Regular,Consolas,monospace; }
.txt-toc-editor__example { margin:0; color:#859188; font-size:12px; line-height:1.4; }
.txt-toc-editor__rule-actions { display:flex; align-items:center; justify-content:flex-end; gap:5px; }
.txt-toc-editor__mini-toggle { gap:5px; padding:0 5px; font-size:12px; white-space:nowrap; }
.txt-toc-editor__mini-toggle input { width:16px; height:16px; }
.txt-toc-editor__scrim { position:absolute; z-index:2; inset:0; display:grid; place-items:center; padding:20px; background:rgb(30 42 34 / 40%); }
.txt-toc-editor__confirm { width:min(420px,100%); padding:22px; border:1px solid #e8ece7; border-radius:13px; background:#fff; box-shadow:0 18px 50px rgb(27 42 31 / 24%); }
.txt-toc-editor__confirm h3 { margin:0; color:#394a3e; font-size:17px; }
.txt-toc-editor__confirm p { margin:9px 0 18px; color:#728076; font-size:14px; line-height:1.6; }
.txt-toc-editor__confirm-actions { display:flex; justify-content:flex-end; gap:9px; margin-top:15px; }
.txt-toc-editor__footer { display:flex; flex:none; align-items:center; justify-content:space-between; gap:12px; padding:12px 26px; border-top:1px solid #edf0ed; color:#8a958d; background:#fbfcfa; font-size:12px; }
@media(max-width:680px) {
  .txt-toc-editor { width:calc(100vw - 20px); max-height:96vh; border-radius:12px; }
  .txt-toc-editor__header { padding:18px 15px 15px; }
  .txt-toc-editor__body { padding:15px; }
  .txt-toc-editor__rule { grid-template-columns:30px minmax(0,1fr); align-items:start; gap:9px; }
  .txt-toc-editor__rule-order { width:27px; height:27px; }
  .txt-toc-editor__rule-actions { grid-column:2; justify-content:flex-start; flex-wrap:wrap; }
  .txt-toc-editor__footer { padding:10px 15px; }
}
</style>

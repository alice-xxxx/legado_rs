<script setup lang="ts">
import { computed, ref } from "vue";
import type { SourceMetadata } from "../../api/types";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import SourceFormEditor from "./SourceFormEditor.vue";

const props = defineProps<{
  source: SourceMetadata;
  jsonText: string;
  originalJson: string;
  busy: boolean;
  locked: boolean;
}>();

const emit = defineEmits<{
  "update:jsonText": [json: string];
  save: [];
  cancel: [];
  pendingDraft: [pending: boolean];
  test: [];
  login: [];
  export: [];
  remove: [];
}>();

const discardPrompt = ref(false);
const fieldPending = ref(false);
const form = ref<InstanceType<typeof SourceFormEditor> | null>(null);
function updatePendingDraft(pending: boolean): void {
  fieldPending.value = pending;
  emit("pendingDraft", pending);
}

const parsed = computed<{ value: Record<string, unknown> | null; error: string }>(() => {
  try {
    const data: unknown = JSON.parse(props.jsonText);
    if (!data || typeof data !== "object" || Array.isArray(data)) {
      return { value: null, error: "必须是 JSON 对象" };
    }
    return { value: data as Record<string, unknown>, error: "" };
  } catch (error) {
    return { value: null, error: error instanceof Error ? error.message : String(error) };
  }
});

const changed = computed(() => props.jsonText !== props.originalJson);

function cancel(): void {
  if (fieldPending.value || changed.value) discardPrompt.value = true;
  else emit("cancel");
}

function reset(): void {
  emit("update:jsonText", props.originalJson);
  form.value?.reset(props.originalJson);
  saveError.value = "";
  discardPrompt.value = false;
}
function save(): void {
  if (props.busy || props.locked) return;
  const json = form.value?.serialize();
  if (!json) return;
  const definition = JSON.parse(json) as Record<string, unknown>;
  if (!String(definition.bookSourceName ?? definition.sourceName ?? definition.name ?? "").trim()
      || !String(definition.bookSourceUrl ?? definition.sourceUrl ?? definition.url ?? "").trim()) {
    saveError.value = "请填写书源名称和地址。";
    return;
  }
  saveError.value = "";
  emit("update:jsonText", json);
  emit("save");
}
const saveError = ref("");
</script>

<template>
 <section class="source-edit-screen">
  <header class="source-edit-bar safe-area-top">
   <button type="button" class="source-edit-back" :disabled="busy" @click="cancel"><PrototypeIcon name="left"/> 返回来源</button>
   <strong>编辑书源</strong>
   <div class="source-edit-tools">
    <button type="button" class="button primary" :disabled="busy||locked||(!changed&&!fieldPending)||!parsed.value" @click="save">{{busy?'保存中…':'保存'}}</button>
    <details class="source-edit-menu">
     <summary aria-label="书源操作"><PrototypeIcon name="more"/></summary>
     <div>
      <button type="button" :disabled="busy||locked||!source.enabled||source.isRss" @click="emit('test')"><PrototypeIcon name="debug"/> 测试配置</button>
      <button v-if="!source.isRss" type="button" :disabled="busy||locked" @click="emit('login')"><PrototypeIcon name="login"/> 登录</button>
      <button type="button" :disabled="busy||locked" @click="emit('export')"><PrototypeIcon name="export"/> 导出</button>
      <button data-source-reset type="button" :disabled="busy||locked||(!changed&&!fieldPending)" @click="reset">还原全部修改</button>
      <button type="button" class="danger" :disabled="busy||locked" @click="emit('remove')"><PrototypeIcon name="trash"/> 删除</button>
      <small v-if="changed||fieldPending">测试、登录和导出使用已保存的配置。</small>
     </div>
    </details>
   </div>
  </header>
  <div class="source-edit-body">
   <p v-if="parsed.error" class="source-edit-error">JSON 格式错误：{{parsed.error}}</p>
   <p v-if="saveError" class="source-edit-error" role="alert">{{saveError}}</p>
   <SourceFormEditor
     ref="form"
     v-if="parsed.value"
     :json-text="jsonText"
     :enabled="source.enabled"
     :disabled="busy||locked"
     @pending="updatePendingDraft"
   />
  </div>
  <div v-if="discardPrompt" class="source-discard-prompt">
    <div><strong>放弃未保存的书源修改？</strong><p>未保存的字段修改将丢失。</p>
      <button type="button" class="button secondary" @click="discardPrompt=false">继续编辑</button>
      <button type="button" class="button danger" @click="emit('cancel')">放弃修改</button>
    </div>
  </div>
 </section>
</template>
<style scoped>
.source-edit-screen{--source-editor-inset:22px;display:flex;flex-direction:column;width:100%;height:100%;min-width:0;min-height:0;padding:0;box-sizing:border-box;margin:0;background:#fff;overflow:hidden}
.source-edit-bar{flex:none;display:grid;grid-template-columns:auto minmax(0,1fr) auto;align-items:center;gap:10px;min-height:58px;padding-inline:var(--source-editor-inset);border-bottom:1px solid var(--app-line)}
.source-edit-back{display:inline-flex;align-items:center;gap:6px;min-height:34px;padding:0 8px;border:0;border-radius:8px;background:transparent;color:#777b86;font-size:12px;cursor:pointer}
.source-edit-back:hover{background:#ece9fb;color:var(--app-accent)}
.source-edit-back :deep(svg){width:16px;height:16px}
.source-edit-bar>strong{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;text-align:center;font-size:16px;color:#414b5b}
.source-edit-tools{display:flex;align-items:center;gap:4px}
.source-edit-tools>.button{background:var(--app-accent);border-radius:8px;font-size:14px;min-height:40px}
.source-edit-menu{position:relative;z-index:8}
.source-edit-menu>summary{display:grid;place-items:center;width:38px;height:44px;cursor:pointer;list-style:none;color:#656e7d}
.source-edit-menu>summary::-webkit-details-marker{display:none}
.source-edit-menu svg{width:20px;height:20px}
.source-edit-menu>div{position:absolute;top:100%;right:0;display:grid;min-width:210px;padding:6px;border:1px solid var(--app-line);border-radius:9px;background:#fff;box-shadow:0 8px 24px #18203422}
.source-edit-menu button{display:flex;align-items:center;gap:9px;min-height:44px;padding:0 12px;border:0;border-radius:6px;background:transparent;text-align:left;color:#485567;font-size:14px;cursor:pointer}
.source-edit-menu button:hover:not(:disabled){background:#f1f4fa}
.source-edit-menu button:disabled{opacity:.4;cursor:default}
.source-edit-menu button.danger{color:#b4515f}
.source-edit-menu small{padding:8px 12px;font-size:12px;line-height:1.5;color:#8b929e}
.source-edit-body{flex:1;min-height:0;overflow-y:auto;width:100%;margin:0;padding:0;overscroll-behavior:contain}
.source-edit-body :deep(.source-field-editor){padding-inline:var(--source-editor-inset)}
.source-edit-error{margin-inline:var(--source-editor-inset);padding:10px 12px;border-radius:9px;font-size:14px;line-height:1.5}
.source-edit-error{color:#b34954;background:#fff0f3}
.source-discard-prompt{position:fixed;z-index:90;inset:0;display:grid;place-items:center;padding:16px;background:#191b2b88}
.source-discard-prompt>div{width:min(440px,100%);box-sizing:border-box;padding:24px;border-radius:14px;background:#fff;box-shadow:0 18px 48px #1617242a}
.source-discard-prompt strong{font-size:17px;color:#31313e}
.source-discard-prompt p{font-size:13px;line-height:1.6;color:#7b7d89}
.source-discard-prompt .button{margin-right:8px}
@media(max-width:860px){.source-edit-screen{--source-editor-inset:14px}.source-edit-body{padding-bottom:calc(18px + env(safe-area-inset-bottom))}.source-edit-back{font-size:0;padding:0 8px}.source-edit-back :deep(svg){width:22px;height:22px}}
</style>

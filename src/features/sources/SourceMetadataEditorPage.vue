<script setup lang="ts">
import { computed, ref } from "vue";
import type { SourceMetadata } from "../../api/types";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import SourceFormEditor from "./SourceFormEditor.vue";

const props = defineProps<{
  source: SourceMetadata;
  jsonText: string;
  originalJson: string;
  originalUrl: string;
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
const urlChanged = computed(() => String(
  parsed.value.value?.bookSourceUrl
  ?? parsed.value.value?.url
  ?? parsed.value.value?.sourceUrl
  ?? "",
) !== props.originalUrl);

function cancel(): void {
  if (fieldPending.value || changed.value) discardPrompt.value = true;
  else emit("cancel");
}

function reset(): void {
  if (fieldPending.value) return;
  emit("update:jsonText", props.originalJson);
  discardPrompt.value = false;
}
</script>

<template>
 <section class="source-edit-screen">
  <header class="source-edit-bar">
   <button type="button" class="source-edit-back" :disabled="busy" @click="cancel"><PrototypeIcon name="left"/> 返回来源</button>
   <strong>编辑书源配置</strong>
   <button type="button" class="button primary" :disabled="busy||locked||!changed||!parsed.value||fieldPending" @click="emit('save')">{{busy?'保存中…':'保存'}}</button>
  </header>
  <div class="source-edit-body">
   <p v-if="fieldPending" class="source-edit-warning">表单有尚未应用的修改，请先应用或撤销后再保存书源。</p>
   <p v-if="urlChanged" class="source-edit-warning">你修改了书源 URL。保存时会保留内部书源 ID，避免现有书籍失去关联。</p>
   <p v-if="parsed.error" class="source-edit-error">JSON 格式错误：{{parsed.error}}</p>
   <SourceFormEditor
     v-if="parsed.value"
     :json-text="jsonText"
     :enabled="source.enabled"
     :disabled="busy||locked"
     @update:json-text="emit('update:jsonText',$event)"
     @pending="updatePendingDraft"
   />
   <section class="source-edit-operations">
      <header><strong>来源操作</strong></header>
      <div>
        <button type="button" :disabled="busy||locked||!source.enabled||source.isRss" @click="emit('test')"><PrototypeIcon name="debug"/> 测试配置</button>
        <button v-if="!source.isRss" type="button" :disabled="busy||locked" @click="emit('login')"><PrototypeIcon name="login"/> 登录</button>
        <button type="button" :disabled="busy||locked" @click="emit('export')"><PrototypeIcon name="export"/> 导出</button>
        <button type="button" class="danger" :disabled="busy||locked" @click="emit('remove')"><PrototypeIcon name="trash"/> 删除</button>
      </div>
      <p v-if="changed||fieldPending">当前还有未保存的草稿。运行测试和导出时使用已保存版本，不会使用草稿。</p>
    </section>
   <footer class="source-edit-actions"><span>{{changed?'有尚未保存的修改':'无修改'}}</span>
    <button v-if="changed" type="button" class="button secondary small" :disabled="busy||fieldPending" @click="reset">还原全部修改</button>
     <button type="button" class="button secondary" :disabled="busy" @click="cancel">取消</button>
    <button type="button" class="button primary" :disabled="busy||locked||!changed||!parsed.value||fieldPending" @click="emit('save')">保存配置</button>
   </footer>
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
.source-edit-screen{width:100%;min-width:0;padding:0 22px 45px;box-sizing:border-box;margin:0;background:#fff}
.source-edit-bar{display:grid;grid-template-columns:auto minmax(0,1fr) auto;align-items:center;gap:10px;min-height:58px;border-bottom:1px solid var(--app-line)}
.source-edit-back{display:inline-flex;align-items:center;gap:6px;min-height:34px;padding:0 8px;border:0;border-radius:8px;background:transparent;color:#777b86;font-size:12px;cursor:pointer}
.source-edit-back:hover{background:#ece9fb;color:var(--app-accent)}
.source-edit-back :deep(svg){width:16px;height:16px}
.source-edit-bar>strong{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;text-align:center;font-size:12px;color:#626573}
.source-edit-bar>.button{justify-self:end;background:var(--app-accent);border-radius:9px;font-size:12px;min-height:35px}
.source-edit-body{width:100%;margin:0;padding:0}
.source-edit-error,.source-edit-warning{padding:10px 12px;border-radius:9px;font-size:12px;line-height:1.5}
.source-edit-error{color:#b34954;background:#fff0f3}
.source-edit-warning{color:#826230;background:#fff8ea}
.source-edit-operations{margin-top:20px;padding:15px 0;border-top:1px solid var(--app-line)}
.source-edit-operations header{display:grid;gap:4px;margin-bottom:12px}
.source-edit-operations header strong{font-size:13px;color:#41434f}
.source-edit-operations>p{font-size:10px;line-height:1.5;color:#9296a2}
.source-edit-operations>div{display:flex;flex-wrap:wrap;gap:8px}
.source-edit-operations>div button{display:inline-flex;align-items:center;gap:6px;min-height:34px;padding:0 11px;border:1px solid #e3e4eb;border-radius:9px;background:#f6f7fa;color:#626875;font-size:11px;cursor:pointer}
.source-edit-operations>div button:hover:not(:disabled){background:#ece9fa;color:var(--app-accent)}
.source-edit-operations>div button.danger{background:#fff5f5;color:#a65a60}
.source-edit-operations>div button:disabled{opacity:.5;cursor:default}
.source-edit-operations>div button :deep(svg){width:15px;height:15px}
.source-edit-actions{position:sticky;bottom:0;z-index:4;display:flex;align-items:center;justify-content:flex-end;gap:8px;margin-top:12px;padding:14px 0;border-top:1px solid var(--app-line);background:#fff}
.source-edit-actions>span{flex:1;color:#888e99;font-size:11px}
.source-edit-actions .button{min-height:36px;border-radius:9px;font-size:12px}
.source-discard-prompt{position:fixed;z-index:90;inset:0;display:grid;place-items:center;padding:16px;background:#191b2b88}
.source-discard-prompt>div{width:min(440px,100%);box-sizing:border-box;padding:24px;border-radius:14px;background:#fff;box-shadow:0 18px 48px #1617242a}
.source-discard-prompt strong{font-size:17px;color:#31313e}
.source-discard-prompt p{font-size:13px;line-height:1.6;color:#7b7d89}
.source-discard-prompt .button{margin-right:8px}
@media(max-width:860px){.source-edit-screen{padding:0 12px 70px}.source-edit-body{padding:17px 0}.source-edit-bar{grid-template-columns:auto minmax(0,1fr) auto}.source-edit-bar>strong{display:none}.source-edit-actions{flex-wrap:wrap}.source-edit-actions>span{flex-basis:100%}}
</style>

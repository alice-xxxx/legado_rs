<script setup lang="ts">
import { computed, ref } from "vue";
import type { SourceMetadata } from "../../api/types";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import SourceFormEditor from "./SourceFormEditor.vue";
import { useOutsidePointerDismiss } from "../../ui/useOutsidePointerDismiss";

const props = defineProps<{
  source?: SourceMetadata;
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
const operationMenu = ref<HTMLDetailsElement | null>(null);

function closeOperationMenu(restoreFocus = false): void {
  const menu = operationMenu.value;
  if (!menu) return;
  menu.open = false;
  if (restoreFocus) menu.querySelector("summary")?.focus({ preventScroll: true });
}

useOutsidePointerDismiss(
  () => Boolean(operationMenu.value?.open),
  target => target instanceof Node && Boolean(operationMenu.value?.contains(target)),
  () => closeOperationMenu(),
);
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
  closeOperationMenu();
  if (fieldPending.value || changed.value) discardPrompt.value = true;
  else emit("cancel");
}

function reset(): void {
  closeOperationMenu();
  emit("update:jsonText", props.originalJson);
  form.value?.reset(props.originalJson);
  saveError.value = "";
  discardPrompt.value = false;
}
function save(): void {
  if (props.busy || props.locked) return;
  closeOperationMenu();
  const json = form.value?.serialize();
  if (!json) return;
  const definition = JSON.parse(json) as Record<string, unknown>;
  if (!String(definition.bookSourceName ?? definition.sourceName ?? definition.name ?? "").trim()
      || !String(definition.bookSourceUrl ?? definition.sourceUrl ?? definition.url ?? "").trim()) {
    saveError.value = "请填写源名称和地址。";
    return;
  }
  saveError.value = "";
  emit("update:jsonText", json);
  emit("save");
}
const saveError = ref("");
</script>

<template>
 <section class="source-edit-screen" data-back-page>
  <header class="source-edit-bar safe-area-top">
   <button type="button" class="source-edit-back" data-back-action data-back-local :disabled="busy" @click="cancel"><PrototypeIcon name="left"/> 返回来源</button>
   <strong>{{ source ? '编辑源' : '新建来源' }}</strong>
   <div class="source-edit-tools">
    <button type="button" class="button primary" :disabled="busy||locked||(!changed&&!fieldPending)||!parsed.value" @click="save">{{busy?'保存中…':'保存'}}</button>
    <details ref="operationMenu" class="source-edit-menu" @keydown.esc.stop.prevent="closeOperationMenu(true)">
     <summary aria-label="源操作"><PrototypeIcon name="more"/></summary>
     <div>
      <template v-if="source">
        <button type="button" :disabled="busy||locked||!source.enabled||source.isRss" @click="closeOperationMenu(); emit('test')"><PrototypeIcon name="debug"/> 测试配置</button>
        <button v-if="!source.isRss" type="button" :disabled="busy||locked" @click="closeOperationMenu(); emit('login')"><PrototypeIcon name="login"/> 登录</button>
        <button type="button" :disabled="busy||locked" @click="closeOperationMenu(); emit('export')"><PrototypeIcon name="export"/> 导出</button>
      </template>
      <button data-source-reset type="button" :disabled="busy||locked||(!changed&&!fieldPending)" @click="reset"><PrototypeIcon name="backup"/> 还原修改</button>
      <template v-if="source">
        <div class="source-edit-menu-divider" role="separator"></div>
        <button type="button" class="danger" :disabled="busy||locked" @click="closeOperationMenu(); emit('remove')"><PrototypeIcon name="trash"/> 删除</button>
        <small v-if="changed||fieldPending">测试、登录和导出使用已保存的配置。</small>
      </template>
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
     :enabled="source?.enabled ?? true"
     :disabled="busy||locked"
     @pending="updatePendingDraft"
   />
  </div>
  <div v-if="discardPrompt" class="source-discard-prompt" data-back-layer>
    <div><strong>放弃未保存的源修改？</strong><p>未保存的字段修改将丢失。</p>
      <button type="button" class="button secondary" data-back-action @click="discardPrompt=false">继续编辑</button>
      <button type="button" class="button danger" @click="emit('cancel')">放弃修改</button>
    </div>
  </div>
 </section>
</template>
<style scoped>
.source-edit-screen{--source-editor-inset:22px;display:flex;flex-direction:column;width:100%;height:100%;min-width:0;min-height:0;padding:0;box-sizing:border-box;margin:0;background:var(--app-surface);overflow:hidden}
.source-edit-bar{flex:none;display:grid;grid-template-columns:auto minmax(0,1fr) auto;align-items:center;gap:10px;min-height:58px;padding-inline:var(--source-editor-inset);border-bottom:1px solid var(--app-line)}
.source-edit-back{display:inline-flex;align-items:center;gap:6px;min-height:34px;padding:0 8px;border:0;border-radius:8px;background:transparent;color:var(--app-muted);font-size:var(--ui-font-caption);cursor:pointer}
.source-edit-back :deep(svg){width:16px;height:16px}
.source-edit-bar>strong{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;text-align:center;font-size:var(--ui-font-body);color:var(--app-text)}
.source-edit-tools{display:flex;align-items:center;gap:4px}
.source-edit-tools>.button{background:var(--app-accent);border-radius:8px;font-size:var(--ui-font-body);min-height:40px}
.source-edit-menu{position:relative;z-index:8}
.source-edit-menu>summary{display:grid;place-items:center;width:36px;height:40px;border-radius:var(--ui-radius);cursor:pointer;list-style:none;color:var(--app-muted)}
.source-edit-menu[open]>summary{background:var(--app-panel-soft);color:var(--app-text)}
.source-edit-menu>summary::-webkit-details-marker{display:none}
.source-edit-menu>summary :deep(svg){width:var(--ui-icon-size);height:var(--ui-icon-size)}
.source-edit-menu>div{position:absolute;top:calc(100% + 4px);right:0;display:grid;gap:2px;width:196px;max-width:calc(100vw - 32px);max-height:min(360px,calc(100dvh - var(--safe-top) - 80px));overflow-y:auto;overscroll-behavior:contain;padding:6px;border:1px solid var(--app-line);border-radius:var(--ui-radius);background:var(--app-surface);box-shadow:var(--app-shadow)}
.source-edit-menu button{display:flex;align-items:center;gap:8px;min-height:36px;padding:0 10px;border:0;border-radius:6px;background:transparent;text-align:left;color:var(--app-text);font-size:var(--ui-font-body);cursor:pointer}
.source-edit-menu button :deep(svg){width:16px;height:16px}
.source-edit-menu-divider{height:1px;margin:4px 6px;background:var(--app-line)}
.source-edit-menu>summary:focus-visible,.source-edit-menu button:focus-visible{outline:2px solid var(--app-accent);outline-offset:-2px}
.source-edit-menu button:disabled{opacity:.4;cursor:default}
.source-edit-menu button.danger{color:var(--app-danger)}
.source-edit-menu small{padding:6px 10px;font-size:var(--ui-font-caption);line-height:1.5;color:var(--app-muted)}
.source-edit-body{flex:1;min-height:0;overflow-y:auto;width:100%;margin:0;padding:0;overscroll-behavior:contain}
.source-edit-body :deep(.source-field-editor){padding-inline:var(--source-editor-inset)}
.source-edit-error{margin-inline:var(--source-editor-inset);padding:10px 12px;border-radius:9px;font-size:var(--ui-font-body);line-height:1.5}
.source-edit-error{color:var(--app-danger);background:var(--app-danger-soft)}
.source-discard-prompt{position:fixed;z-index:90;inset:0;display:grid;place-items:center;padding:16px;background:#191b2b88}
.source-discard-prompt>div{width:min(440px,100%);box-sizing:border-box;padding:24px;border-radius:14px;background:var(--app-surface);box-shadow:0 18px 48px #1617242a}
.source-discard-prompt strong{font-size:var(--ui-font-heading);color:var(--app-text)}
.source-discard-prompt p{font-size:var(--ui-font-caption);line-height:1.6;color:var(--app-muted)}
.source-discard-prompt .button{margin-right:8px}
@media(max-width:860px){.source-edit-screen{--source-editor-inset:14px}.source-edit-body{padding-bottom:calc(18px + var(--safe-bottom))}.source-edit-back{font-size:0;padding:0 8px}.source-edit-back :deep(svg){width:22px;height:22px}}
</style>

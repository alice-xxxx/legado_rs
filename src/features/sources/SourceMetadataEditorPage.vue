<script setup lang="ts">
import SubpageHeader from "../../ui/SubpageHeader.vue";
import { Button as KButton, Popover as KPopover } from "konsta/vue";
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

const operationMenuOpen = ref(false);
const discardPrompt = ref(false);
const fieldPending = ref(false);
const form = ref<InstanceType<typeof SourceFormEditor> | null>(null);
const operationMenu = ref<HTMLDetailsElement | null>(null);

function closeOperationMenu(restoreFocus = false): void {
  const menu = operationMenu.value;
  if (!menu) return;
  menu.open = false;
  operationMenuOpen.value = false;
  if (restoreFocus) menu.querySelector("summary")?.focus({ preventScroll: true });
}

useOutsidePointerDismiss(
  () => operationMenuOpen.value,
  target => target instanceof Node && Boolean(operationMenu.value?.contains(target) || target instanceof Element && target.closest(".source-edit-menu-popover")),
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
  <SubpageHeader :title="source ? '编辑源' : '新建来源'" back-label="返回来源" :back-disabled="busy" local-back @back="cancel"><template #actions><div class="source-edit-tools">
    <KButton rounded inline type="button" class="gap-2" :disabled="busy||locked||(!changed&&!fieldPending)||!parsed.value" @click="save">{{busy?'保存中…':'保存'}}</KButton>
    <details @toggle="operationMenuOpen = ($event.target as HTMLDetailsElement).open" ref="operationMenu" class="source-edit-menu" @keydown.esc.stop.prevent="closeOperationMenu(true)">
     <summary aria-label="源操作"><PrototypeIcon name="more"/></summary>

    </details>
   </div></template></SubpageHeader>
<KPopover v-if="operationMenuOpen" opened :backdrop="false" :target="operationMenu ?? undefined" class="source-edit-menu-popover">
  <div class="p-2">
<template v-if="source">
        <KButton rounded inline clear class="gap-2 w-full justify-start text-current" type="button" :disabled="busy||locked||!source.enabled||source.isRss" @click="closeOperationMenu(); emit('test')"><PrototypeIcon name="debug"/> 测试配置</KButton>
        <KButton rounded inline clear class="gap-2 w-full justify-start text-current" v-if="!source.isRss" type="button" :disabled="busy||locked" @click="closeOperationMenu(); emit('login')"><PrototypeIcon name="login"/> 登录</KButton>
        <KButton rounded inline clear class="gap-2 w-full justify-start text-current" type="button" :disabled="busy||locked" @click="closeOperationMenu(); emit('export')"><PrototypeIcon name="export"/> 导出</KButton>
      </template><KButton rounded inline clear class="gap-2 w-full justify-start text-current" data-source-reset type="button" :disabled="busy||locked||(!changed&&!fieldPending)" @click="reset"><PrototypeIcon name="backup"/> 还原修改</KButton><template v-if="source">
        <div class="source-edit-menu-divider" role="separator"></div>
        <KButton rounded inline type="button" class="k-color-brand-danger gap-2 " :disabled="busy||locked" @click="closeOperationMenu(); emit('remove')"><PrototypeIcon name="trash"/> 删除</KButton>
        <small v-if="changed||fieldPending">测试、登录和导出使用已保存的配置。</small>
      </template>
  </div>
</KPopover>
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
      <KButton rounded inline tonal type="button" class="gap-2" data-back-action @click="discardPrompt=false">继续编辑</KButton>
      <KButton rounded inline type="button" class="k-color-brand-danger gap-2 " @click="emit('cancel')">放弃修改</KButton>
    </div>
  </div>
 </section>
</template>
<style scoped>
@layer components {
.source-edit-screen{--source-editor-inset:22px;display:flex;flex-direction:column;width:100%;height:100%;min-width:0;min-height:0;padding:0;box-sizing:border-box;margin:0;background:var(--app-background);overflow:hidden}
.source-edit-tools{display:flex;align-items:center;gap:4px}
.source-edit-menu{position:relative;z-index:8}
.source-edit-menu>summary{display:grid;place-items:center;width:36px;height:40px;border-radius:var(--ui-radius);cursor:pointer;list-style:none;color:var(--app-muted)}
.source-edit-menu[open]>summary{background:var(--app-panel-soft);color:var(--app-text)}
.source-edit-menu>summary::-webkit-details-marker{display:none}
.source-edit-menu>summary :deep(svg){width:var(--ui-icon-size);height:var(--ui-icon-size)}
.source-edit-menu-divider{height:1px;margin:4px 6px;background:var(--app-line)}
.source-edit-menu>summary:focus-visible,.source-edit-menu button:focus-visible{outline:2px solid var(--app-accent);outline-offset:-2px}
.source-edit-body{flex:1;min-height:0;overflow-y:auto;width:100%;margin:0;padding:0;overscroll-behavior:contain}
.source-edit-body :deep(.source-field-editor){padding-inline:var(--source-editor-inset)}
.source-edit-error{margin-inline:var(--source-editor-inset);padding:10px 12px;border-radius:9px;font-size:var(--ui-font-body);line-height:1.5}
.source-edit-error{color:var(--app-danger);background:var(--app-danger-soft)}
.source-discard-prompt{position:fixed;z-index:90;inset:0;display:grid;place-items:center;padding:16px;background:var(--app-scrim)}
.source-discard-prompt>div{width:min(440px,100%);box-sizing:border-box;padding:24px;border-radius:14px;background:var(--app-surface);box-shadow:0 18px 48px #1617242a}
.source-discard-prompt strong{font-size:var(--ui-font-heading);color:var(--app-text)}
.source-discard-prompt p{font-size:var(--ui-font-caption);line-height:1.6;color:var(--app-muted)}
@media(max-width:860px){.source-edit-screen{--source-editor-inset:var(--ui-page-gutter)}.source-edit-body{padding-bottom:calc(18px + var(--safe-bottom))}}
}
</style>

<script setup lang="ts">
// 书籍展示信息编辑器：收集用户修改并调用应用层的封面选择能力。
import { computed, onBeforeUnmount, reactive, ref } from "vue";
import { onBeforeRouteLeave } from "vue-router";
import { discardBookCoverAsset, pickBookCover } from "../../api/books";
import type { BookDisplayMetadataPatch, BookResource } from "../../api/types";
import BackButton from "../../ui/BackButton.vue";

type MetadataField = "title" | "author" | "intro" | "coverSrc";

const props = defineProps<{
  book: BookResource;
  busy: boolean;
  locked: boolean;
  error: string;
  returnLabel: string;
}>();
const emit = defineEmits<{
  close: [];
  save: [patch: BookDisplayMetadataPatch];
}>();

const fields: MetadataField[] = ["title", "author", "intro", "coverSrc"];
const bookAtOpen = props.book;
const base = {
  title: bookAtOpen.displayBase?.title ?? bookAtOpen.title,
  author: bookAtOpen.displayBase?.author ?? "",
  intro: bookAtOpen.displayBase?.intro ?? "",
  coverSrc: bookAtOpen.displayBase?.coverSrc ?? "",
};
const initial = {
  title: bookAtOpen.title,
  author: bookAtOpen.author ?? "",
  intro: bookAtOpen.intro ?? "",
  coverSrc: bookAtOpen.coverSrc ?? "",
};
const initialOverrides = { ...(bookAtOpen.displayOverrides ?? {}) };
const draft = reactive({ ...initial });
const coverPreviewSrc = ref(initial.coverSrc.trim());
const coverPreviewError = ref(false);
const coverPreviewMessage = ref("");
const coverPicking = ref(false);
const coverSelectionError = ref("");
const selectedCoverAssetId = ref<string | null>(null);
const selectedCoverRef = ref("");
const selectedCoverPreviewSrc = ref("");
const discardPromptOpen = ref(false);
let pendingRouteNavigation: ((allow: boolean) => void) | null = null;
const resetFields = reactive<Record<MetadataField, boolean>>({
  title: false,
  author: false,
  intro: false,
  coverSrc: false,
});

const hasChanges = computed(() => fields.some((field) =>
  resetFields[field] || draft[field] !== initial[field]));

function hasOverride(field: MetadataField): boolean {
  return Object.prototype.hasOwnProperty.call(initialOverrides, field);
}

function resetField(field: MetadataField): void {
  draft[field] = hasOverride(field) ? base[field] : initial[field];
  resetFields[field] = hasOverride(field);
}

function resetAll(): void {
  for (const field of fields) resetField(field);
  previewCover();
}

function clearCoverPreview(): void {
  coverPreviewSrc.value = "";
  coverPreviewError.value = false;
  coverPreviewMessage.value = "";
}

function previewCover(): void {
  const value = draft.coverSrc.trim();
  coverPreviewError.value = false;
  coverPreviewMessage.value = "";
  if (!value) {
    coverPreviewSrc.value = "";
    coverPreviewMessage.value = "请输入封面地址后预览。";
    return;
  }
  if (value === selectedCoverRef.value) {
    if (selectedCoverPreviewSrc.value) {
      coverPreviewSrc.value = selectedCoverPreviewSrc.value;
      return;
    }
    coverPreviewSrc.value = "";
    coverPreviewMessage.value = "所选本地封面会在保存后由应用读取并显示。";
    return;
  }
  if (value !== initial.coverSrc.trim() && !isSafeExternalCoverUrl(value)) {
    coverPreviewSrc.value = "";
    coverPreviewMessage.value = "预览仅支持 HTTPS 封面地址或当前书籍已有封面。";
    return;
  }
  coverPreviewSrc.value = value;
}

async function discardSelectedCoverAsset(): Promise<void> {
  const assetId = selectedCoverAssetId.value;
  if (!assetId) return;
  selectedCoverAssetId.value = null;
  selectedCoverRef.value = "";
  selectedCoverPreviewSrc.value = "";
  try {
    await discardBookCoverAsset(props.book.id, assetId);
  } catch {
    // Leave an unreferenced orphan rather than risk changing a saved cover.
  }
}

async function chooseLocalCover(): Promise<void> {
  if (props.busy || props.locked || coverPicking.value) return;
  coverPicking.value = true;
  coverSelectionError.value = "";
  try {
    const selection = await pickBookCover(props.book.id);
    if (selection.cancelled) return;
    await discardSelectedCoverAsset();
    selectedCoverAssetId.value = selection.assetId;
    selectedCoverRef.value = selection.coverSrc;
    selectedCoverPreviewSrc.value = selection.previewSrc;
    draft.coverSrc = selection.coverSrc;
    resetFields.coverSrc = false;
    coverPreviewSrc.value = selection.previewSrc;
    coverPreviewError.value = false;
    coverPreviewMessage.value = "";
  } catch (error) {
    coverSelectionError.value = error instanceof Error ? error.message : String(error);
  } finally {
    coverPicking.value = false;
  }
}

onBeforeRouteLeave(() => {
  if (props.busy || coverPicking.value) return false;
  if (!hasChanges.value) return true;
  return new Promise<boolean>(resolve => {
    pendingRouteNavigation?.(false);
    pendingRouteNavigation = resolve;
    discardPromptOpen.value = true;
  });
});
onBeforeUnmount(() => {
  pendingRouteNavigation?.(false);
  void discardSelectedCoverAsset();
});
function requestClose(): void {
  if (props.busy || coverPicking.value) return;
  if (hasChanges.value) discardPromptOpen.value = true;
  else emit("close");
}
function keepEditing(): void {
  discardPromptOpen.value = false;
  const resume = pendingRouteNavigation;
  pendingRouteNavigation = null;
  resume?.(false);
}
function discardAndClose(): void {
  if (props.busy || coverPicking.value) return;
  discardPromptOpen.value = false;
  const resume = pendingRouteNavigation;
  pendingRouteNavigation = null;
  if (resume) resume(true);
  else emit("close");
}

function isSafeExternalCoverUrl(value: string): boolean {
  try {
    const url = new URL(value);
    const host = url.hostname.toLowerCase();
    const ipv4 = host.split(".").map(Number);
    const loopbackIpv4 = ipv4.length === 4 && ipv4.every((part) => Number.isInteger(part) && part >= 0 && part <= 255) && ipv4[0] === 127;
    return url.protocol === "https:" && Boolean(host) && !url.username && !url.password &&
      host !== "localhost" && host !== "[::1]" && !loopbackIpv4;
  } catch {
    return false;
  }
}

function submit(): void {
  if (props.busy || coverPicking.value || !hasChanges.value) return;
  const patch: BookDisplayMetadataPatch = {};
  for (const field of fields) {
    if (resetFields[field]) patch[field] = null;
    else if (draft[field] !== initial[field]) patch[field] = draft[field];
  }
  if (Object.keys(patch).length) emit("save", patch);
}
</script>

<template>
  <Teleport to="body">
    <section class="book-display-editor-backdrop" @click.self="requestClose">
      <article class="book-display-editor">
        <header class="book-display-editor-toolbar">
          <BackButton class="book-display-editor-back" :label="returnLabel" :disabled="busy || coverPicking" @click="requestClose" />
          <h2 id="book-display-editor-title">书籍信息编辑</h2>
          <button type="submit" form="book-display-editor-form" class="book-display-editor-save" :title="busy ? '正在保存' : '保存书籍信息'" :disabled="busy || locked || coverPicking || !hasChanges">{{ busy ? '保存中…' : '保存' }}</button>
        </header>
        <form id="book-display-editor-form" class="book-display-editor-form" @submit.prevent="submit">
          <div class="book-display-editor-identity">
            <div class="book-display-cover-preview">
              <img v-if="coverPreviewSrc && !coverPreviewError" :src="coverPreviewSrc" @error="coverPreviewError = true" />
              <span v-else-if="coverPreviewError">封面无法加载，请检查地址或资源是否可用。</span>
              <span v-else>{{ coverPreviewMessage || "暂无封面" }}</span>
            </div>
            <div class="book-display-identity-fields">
              <label class="book-display-field">
                <span>书名</span>
                <div class="book-display-input-row">
                  <input v-model="draft.title" autocomplete="off" :disabled="busy || locked" @input="resetFields.title = false" />
                </div>
              </label>

              <label class="book-display-field">
                <span>作者</span>
                <div class="book-display-input-row">
                  <input v-model="draft.author" autocomplete="off" :disabled="busy || locked" @input="resetFields.author = false" />
                </div>
              </label>
            </div>
          </div>

          <label class="book-display-field">
            <span>简介</span>
            <textarea v-model="draft.intro" rows="5" :disabled="busy || locked" @input="resetFields.intro = false"></textarea>
          </label>

          <label class="book-display-field">
            <span>封面地址</span>
            <div class="book-display-input-row">
              <input v-model="draft.coverSrc" autocomplete="off" inputmode="url" :disabled="busy || locked || coverPicking" @input="resetFields.coverSrc = false; clearCoverPreview()" />
              <button type="button" class="text-button" :disabled="busy || locked || coverPicking" @click="chooseLocalCover">{{ coverPicking ? "正在选择…" : "选择本地图片" }}</button>
              <button type="button" class="text-button" :disabled="busy || locked || coverPicking || !draft.coverSrc.trim()" @click="previewCover">预览</button>
            </div>
            <small>支持选择 JPEG、PNG、WebP 图片或填写 HTTPS 地址；留空表示不显示封面。所选图片在保存显示信息后生效。</small>
            <p v-if="coverSelectionError" class="book-display-editor-error">{{ coverSelectionError }}</p>
          </label>

          <button type="button" class="text-button book-display-reset-all" :disabled="busy || locked" @click="resetAll">全部恢复原值</button>
          <p v-if="error" class="book-display-editor-error">{{ error }}</p>
          <p v-else-if="locked" class="book-display-editor-error">恢复完成前不能继续保存。请重启应用后检查书架。</p>
          <footer class="book-display-editor-actions">
            <button type="button" class="button secondary" :disabled="busy || coverPicking" @click="requestClose">取消</button>
            <button type="submit" class="button primary" :disabled="busy || locked || coverPicking || !hasChanges">{{ busy ? '正在保存…' : '保存修改' }}</button>
          </footer>
        </form>
      </article>
      <div v-if="discardPromptOpen" class="book-display-discard-scrim">
        <div class="book-display-discard-dialog">
          <h3 id="book-display-discard-title">放弃未保存的书籍信息？</h3>
          <p>书名、作者、简介或封面尚有修改，离开后这些修改将不会保存。</p>
          <footer>
            <button type="button" class="button secondary" @click="keepEditing">继续编辑</button>
            <button type="button" class="button danger" @click="discardAndClose">放弃修改</button>
          </footer>
        </div>
      </div>
    </section>
  </Teleport>
</template>

<style scoped>
.book-display-editor-backdrop{--be-accent:var(--app-accent);--be-line:var(--app-line);--be-muted:#959aa5;position:fixed;z-index:80;inset:0;display:grid;place-items:center;padding:18px;background:#191b2780;backdrop-filter:blur(2px)}
.book-display-editor{width:min(700px,100%);height:min(860px,92dvh);display:flex;flex-direction:column;overflow:hidden;border:1px solid var(--be-line);border-radius:15px;color:#343640;background:#fff;box-shadow:0 24px 72px #15172130}
.book-display-editor-toolbar{flex:none;min-height:61px;display:grid;grid-template-columns:auto minmax(0,1fr) auto;align-items:center;gap:12px;padding:6px 20px;border-bottom:1px solid var(--be-line)}
.book-display-editor-toolbar h2{margin:0;text-align:center;color:#343640;font-size:16px;overflow:hidden;white-space:nowrap;text-overflow:ellipsis}
.book-display-editor-back{justify-self:start}
.book-display-editor-save{min-height:35px;padding:0 14px;border:0;border-radius:9px;background:var(--be-accent);color:#fff;font-size:12px;font-weight:700;cursor:pointer}
.book-display-editor-save:disabled{opacity:.5;cursor:default}
.book-display-editor-form{min-height:0;display:grid;align-content:start;gap:20px;overflow:auto;padding:20px 24px 30px}
.book-display-editor-identity{display:grid;grid-template-columns:112px minmax(0,1fr);align-items:start;gap:17px}
.book-display-identity-fields{display:grid;gap:14px;min-width:0}
.book-display-field{display:grid;gap:7px;min-width:0;color:#5f6370;font-size:12px;font-weight:650}
.book-display-field input,.book-display-field textarea{box-sizing:border-box;min-width:0;width:100%;padding:10px 11px;border:1px solid #dfe3ea;border-radius:9px;color:#343640;background:#fff;font-size:13px;font-family:inherit;line-height:1.55}
.book-display-field input{min-height:40px}
.book-display-field textarea{min-height:130px;resize:vertical;white-space:pre-wrap;overflow-wrap:anywhere}
.book-display-field input:focus,.book-display-field textarea:focus{outline:2px solid #6757d644;outline-offset:0;border-color:#a99ce9}
.book-display-field small{color:var(--be-muted);font-size:11px;line-height:1.5;font-weight:400;overflow-wrap:anywhere}
.book-display-input-row{display:flex;align-items:center;flex-wrap:wrap;gap:8px;min-width:0}
.book-display-input-row input{flex:1 1 200px}
.book-display-input-row .text-button{flex:none;min-height:35px;padding:0 8px;border:0;border-radius:8px;color:var(--be-accent);background:#f1effa;font-size:11px;white-space:nowrap;cursor:pointer}
.book-display-input-row .text-button:disabled{opacity:.5;cursor:default}
.book-display-cover-preview{width:112px;height:154px;display:grid;place-items:center;overflow:hidden;border:1px solid var(--be-line);border-radius:9px;color:#999da7;background:#f5f6f9;font-size:11px;line-height:1.5;text-align:center}
.book-display-cover-preview img{width:100%;height:100%;object-fit:cover}
.book-display-cover-preview span{padding:8px}
.book-display-reset-all{justify-self:start;min-height:32px;padding:0 5px;border:0;background:transparent;color:var(--be-accent);font-size:12px;cursor:pointer}
.book-display-editor-error{margin:0;padding:10px 12px;border:1px solid #efd6da;border-radius:9px;color:#b05763;background:#fff1f3;font-size:12px;line-height:1.55;overflow-wrap:anywhere}
.book-display-editor-actions{position:sticky;bottom:-30px;z-index:2;display:flex;align-items:center;justify-content:flex-end;flex-wrap:wrap;gap:8px;margin:0 -24px -30px;padding:14px 24px max(14px,env(safe-area-inset-bottom));border-top:1px solid var(--be-line);background:#fff}
.book-display-editor-actions .button{min-height:36px;white-space:nowrap;font-size:12px}
.book-display-discard-scrim{position:fixed;z-index:90;inset:0;display:grid;place-items:center;padding:14px;background:#191b2799}
.book-display-discard-dialog{box-sizing:border-box;width:min(435px,100%);padding:24px;border:1px solid var(--be-line);border-radius:14px;background:#fff;box-shadow:0 18px 55px #15172133}
.book-display-discard-dialog h3{margin:0;font-size:17px;color:#343640}
.book-display-discard-dialog p{margin:11px 0;color:#777d87;font-size:13px;line-height:1.65}
.book-display-discard-dialog footer{display:flex;justify-content:flex-end;gap:9px;margin-top:19px}
@media(max-width:640px){
 .book-display-editor-backdrop{padding:0;display:block}
 .book-display-editor{width:100%;height:100dvh;border:0;border-radius:0}
 .book-display-editor-toolbar{min-height:calc(55px + env(safe-area-inset-top));padding:env(safe-area-inset-top) 12px 0}
 .book-display-editor-toolbar h2{font-size:14px}
 .book-display-editor-form{gap:17px;padding:16px 15px 22px}
 .book-display-editor-identity{grid-template-columns:92px minmax(0,1fr);gap:12px}
 .book-display-cover-preview{width:92px;height:127px}
 .book-display-identity-fields{gap:11px}
 .book-display-input-row input{flex-basis:100%}
 .book-display-editor-actions{margin:0 -15px -22px;bottom:-22px;padding:11px 15px max(14px,env(safe-area-inset-bottom))}
}
</style>

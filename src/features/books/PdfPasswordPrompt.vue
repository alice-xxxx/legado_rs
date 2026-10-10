<script setup lang="ts">
type PdfImportPromptContext = {
  pdfPasswordPromptOpen: boolean;
  pdfPasswordInput: string;
  pdfPasswordBusy: boolean;
  pdfPasswordError: string;
  submitPdfPassword: () => void | Promise<void>;
  cancelPdfPasswordPrompt: () => void;
};

defineProps<{ context: PdfImportPromptContext }>();
</script>

<template>
  <section
    v-if="context.pdfPasswordPromptOpen"
    class="modal-backdrop"
    @click.self="!context.pdfPasswordBusy && context.cancelPdfPasswordPrompt()"
  >
    <article class="app-modal pdf-password-modal">
      <p class="eyebrow">打开 PDF</p>
      <h2>输入文件密码</h2>
      <p class="modal-description">此文件受密码保护。密码仅用于当前导入过程，不会保存。</p>
      <form @submit.prevent="context.submitPdfPassword">
        <label class="form-field">
          <span>PDF 密码</span>
          <input v-model="context.pdfPasswordInput" type="password" autocomplete="off" autofocus :disabled="context.pdfPasswordBusy" />
        </label>
        <p v-if="context.pdfPasswordError" class="pdf-password-error">{{ context.pdfPasswordError }}</p>
        <div class="modal-actions">
          <button type="button" class="button secondary" :disabled="context.pdfPasswordBusy" @click="context.cancelPdfPasswordPrompt">取消</button>
          <button type="submit" class="button primary" :disabled="context.pdfPasswordBusy || !context.pdfPasswordInput">
            {{ context.pdfPasswordBusy ? '正在打开…' : '打开文件' }}
          </button>
        </div>
      </form>
    </article>
  </section>
</template>

<style scoped>
.modal-backdrop { position: fixed; z-index: 50; inset: 0; display: flex; align-items: center; justify-content: center; padding: 16px; background: rgb(25 27 35 / 36%); backdrop-filter: blur(3px); }
.app-modal { width: min(560px, 100%); max-height: min(88dvh, 900px); overflow: auto; padding: 22px; border: 1px solid var(--app-line); border-radius: 16px; background: var(--app-surface); box-shadow: var(--app-shadow); }
.eyebrow { margin: 0 0 6px; color: var(--app-muted); font-size: 11px; }
h2 { margin: 0 0 10px; font-size: 18px; }
.modal-description { color: var(--app-muted); font-size: 13px; line-height: 1.55; }
.form-field { display: grid; gap: 7px; margin-top: 18px; font-size: 12px; }
.form-field input { min-height: 40px; padding: 0 10px; border: 1px solid var(--app-line); border-radius: 9px; background: var(--app-surface); }
.pdf-password-error { color: var(--app-danger); font-size: 12px; }
.modal-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 16px; }
</style>

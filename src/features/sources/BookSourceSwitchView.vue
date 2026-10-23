<script setup lang="ts">
import BackButton from "../../ui/BackButton.vue";
import BookSourcesPanel from "./BookSourcesPanel.vue";
import type { BookSourceSwitchSession } from "./bookSourceSwitchSession";
defineProps<{ session: BookSourceSwitchSession; locked: boolean }>();
</script>

<template>
  <section v-if="session.bookSourceSwitchOpen && !session.bookSourceSwitchInline && session.bookSourceSwitchBook" class="book-source-switch-page" data-back-layer @click.stop @keydown.esc.stop.prevent="session.closeBookSourceSwitch">
    <header class="source-page-header">
      <BackButton label="返回阅读" @click="session.closeBookSourceSwitch" />
      <div><strong>来源</strong><small>{{ session.bookSourceSwitchBook.title }}</small></div>
    </header>
    <main class="source-page-content">
      <BookSourcesPanel :book="session.bookSourceSwitchBook" :candidates="session.bookSourceCandidateResults" :session="session" :locked="locked"
        :can-check="session.bookSourceSwitchBook.canChangeSource === true && !!session.bookSourceSwitchBook.sourceId && !!session.bookSourceSwitchBook.bookUrl"
        :check-busy="session.bookSourceCheckBusy"
        @select="session.chooseBookSourceCandidate" @search="session.startBookSourceCandidateSearch" @check="session.checkBookSourceUpdates" />
    </main>
  </section>
  <section v-if="session.bookSourceSwitchConfirmOpen && session.selectedBookSourceCandidate" class="modal-backdrop source-confirm-backdrop"
    @click.self="!session.bookSourceSwitchBusy && (session.bookSourceSwitchConfirmOpen = false)"
    @keydown.esc.stop.prevent="!session.bookSourceSwitchBusy && (session.bookSourceSwitchConfirmOpen = false)">
    <article class="app-modal source-confirm" role="dialog" aria-modal="true" aria-labelledby="source-confirm-title">
      <h2 id="source-confirm-title">更换来源？</h2>
      <p>将《{{ session.bookSourceSwitchBook?.title }}》切换到“{{ session.selectedBookSourceCandidate.sourceName }}”。</p>
      <p>阅读位置会匹配新目录，原有书签会保留。</p>
      <p v-if="session.bookSourceChangeError" class="source-identity-warning">{{ session.bookSourceChangeError }}</p>
      <div class="modal-actions">
        <button type="button" class="button secondary small" :disabled="session.bookSourceSwitchBusy && !session.bookSourceSwitchCancellable" data-back-action @click="session.bookSourceSwitchCancellable ? session.closeBookSourceSwitch() : (session.bookSourceSwitchConfirmOpen = false)">{{ session.bookSourceSwitchCancellable ? '取消换源' : '返回' }}</button>
        <button type="button" class="button primary small" :disabled="session.bookSourceSwitchBusy || locked" @click="session.confirmBookSourceChange">{{ session.bookSourceSwitchBusy ? '正在更换…' : '确认更换' }}</button>
      </div>
    </article>
  </section>
</template>

<style scoped>
.book-source-switch-page { position: fixed; z-index: 60; inset: 0; display: flex; flex-direction: column; pointer-events: auto; color: var(--app-text); background: var(--app-background); }
.source-page-header { display: flex; align-items: center; flex: none; gap: 12px; width: min(920px, 100%); min-height: calc(64px + var(--safe-top)); margin-inline: auto; padding: calc(8px + var(--safe-top)) 20px 8px; border-bottom: 1px solid var(--app-line); }
.source-page-header > div { display: grid; gap: 3px; min-width: 0; }
.source-page-header strong { font-size: var(--ui-font-body); font-weight: 600; }
.source-page-header small { overflow: hidden; color: var(--app-muted); font-size: var(--ui-font-caption); text-overflow: ellipsis; white-space: nowrap; }
.source-page-content { flex: 1; width: min(920px, 100%); min-height: 0; margin-inline: auto; overflow: auto; padding: var(--ui-space-2) var(--ui-page-gutter) max(24px, var(--safe-bottom)); overscroll-behavior: contain; }
.source-confirm-backdrop { z-index: 61; pointer-events: auto; }
.source-confirm { width: min(420px, 100%); padding: 24px; color: var(--app-text); }
.source-confirm h2 { margin: 0 0 12px; font-size: var(--ui-font-heading); }
.source-confirm p { margin: 8px 0; color: var(--app-muted); font-size: var(--ui-font-body); line-height: 1.7; overflow-wrap: anywhere; }
.source-confirm .source-identity-warning { color: var(--app-warning); }
.source-confirm .modal-actions { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; margin-top: 24px; }
.source-confirm .button { min-height: 44px; padding-inline: 8px; }
@media (max-width: 600px) {
  .source-page-header { min-height: calc(56px + var(--safe-top)); padding-inline: 12px; }
}
</style>

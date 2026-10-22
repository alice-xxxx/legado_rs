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
      <h2 id="source-confirm-title">确认更换源？</h2>
      <p>将《{{ session.bookSourceSwitchBook?.title }}》切换到“{{ session.selectedBookSourceCandidate.sourceName }}”。阅读位置会按新目录更新；无法匹配的书签仍会保留。</p>
      <p v-if="session.bookSourceChangeError" class="source-identity-warning">{{ session.bookSourceChangeError }}</p>
      <div class="modal-actions">
        <button type="button" class="button secondary small" :disabled="session.bookSourceSwitchBusy" data-back-action @click="session.bookSourceSwitchConfirmOpen = false">返回</button>
        <button type="button" class="button primary small" :disabled="session.bookSourceSwitchBusy || locked" @click="session.confirmBookSourceChange">{{ session.bookSourceSwitchBusy ? '正在更换…' : '确认更换' }}</button>
      </div>
    </article>
  </section>
</template>

<style scoped>
.book-source-switch-page { display: flex; position: fixed; z-index: 60; inset: 0; min-height: 0; flex-direction: column; pointer-events: auto; color: var(--app-text); background: var(--app-background); }
.source-page-header { display: flex; flex: none; align-items: center; gap: 12px; padding: calc(8px + var(--safe-top)) 12px 8px; border-bottom: 1px solid var(--app-line); background: var(--app-surface); }
.source-page-header > div { display: grid; min-width: 0; gap: 3px; }
.source-page-header strong { font-size: var(--ui-font-body); }
.source-page-header small { overflow: hidden; color: var(--app-muted); font-size: var(--ui-font-caption); text-overflow: ellipsis; white-space: nowrap; }
.source-page-content { min-height: 0; flex: 1; overflow: auto; padding: 22px 16px calc(22px + var(--safe-bottom)); overscroll-behavior: contain; }
.source-confirm-backdrop { z-index: 61; pointer-events: auto; }
.source-confirm { width: min(420px, 100%); color: var(--app-text); }
.source-confirm h2 { margin: 0 0 12px; font-size: var(--ui-font-heading); }
.source-confirm p { font-size: var(--ui-font-body); line-height: 1.6; overflow-wrap: anywhere; }
.source-identity-warning { color: var(--app-warning); }
.source-confirm button:focus-visible { outline: 2px solid var(--app-accent); outline-offset: 2px; }
</style>

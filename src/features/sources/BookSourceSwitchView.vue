<script setup lang="ts">
import { Dialog as KDialog, DialogButton as KDialogButton } from "konsta/vue";
import SubpageHeader from "../../ui/SubpageHeader.vue";
import BookSourcesPanel from "./BookSourcesPanel.vue";
import type { BookSourceSwitchSession } from "./bookSourceSwitchSession";
defineProps<{ session: BookSourceSwitchSession; locked: boolean }>();
</script>

<template>
  <section v-if="session.bookSourceSwitchOpen && !session.bookSourceSwitchInline && session.bookSourceSwitchBook" class="book-source-switch-page" data-back-layer @click.stop @keydown.esc.stop.prevent="session.closeBookSourceSwitch">
    <SubpageHeader title="来源" :subtitle="session.bookSourceSwitchBook.title" back-label="返回阅读" local-back @back="session.closeBookSourceSwitch" />
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
    <KDialog opened :backdrop="false" class="source-confirm w-[min(420px,calc(100vw-32px))] max-h-[88dvh] overflow-auto">
  <template #title><h2 id="source-confirm-title">更换来源？</h2></template>
<p>将《{{ session.bookSourceSwitchBook?.title }}》切换到“{{ session.selectedBookSourceCandidate.sourceName }}”。</p>
<p>阅读位置会匹配新目录，原有书签会保留。</p>
<p v-if="session.bookSourceChangeError" class="source-identity-warning">{{ session.bookSourceChangeError }}</p><template #buttons><KDialogButton type="button" :disabled="session.bookSourceSwitchBusy && !session.bookSourceSwitchCancellable" data-back-action @click="session.bookSourceSwitchCancellable ? session.closeBookSourceSwitch() : (session.bookSourceSwitchConfirmOpen = false)">{{ session.bookSourceSwitchCancellable ? '取消换源' : '返回' }}</KDialogButton>
<KDialogButton type="button" :disabled="session.bookSourceSwitchBusy || locked" @click="session.confirmBookSourceChange" strong>{{ session.bookSourceSwitchBusy ? '正在更换…' : '确认更换' }}</KDialogButton></template>
</KDialog>
  </section>
</template>

<style scoped>
@layer components {
.book-source-switch-page { position: fixed; z-index: 60; inset: 0; display: flex; flex-direction: column; pointer-events: auto; color: var(--app-text); background: var(--app-background); }
.source-page-content { flex: 1; width: min(var(--app-content-width), 100%); min-height: 0; margin-inline: auto; overflow: auto; padding: var(--ui-space-2) var(--ui-page-gutter) max(24px, var(--safe-bottom)); overscroll-behavior: contain; }
.source-confirm-backdrop { z-index: 61; pointer-events: auto; }

}
</style>

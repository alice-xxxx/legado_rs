<script setup lang="ts">
import { toRefs } from "vue";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import type { BookResource, SourceMetadata } from "../../api/types";
import type { BookSourceSwitchSession } from "./bookSourceSwitchSession";

const props = defineProps<{
  bookSourceSwitch: BookSourceSwitchSession;
  bookSourceCandidates: SourceMetadata[];
  sourceName: (id: string) => string;
  shelfBatchRecoveryRequired: boolean;
}>();
const { bookSourceSwitch, bookSourceCandidates, sourceName, shelfBatchRecoveryRequired } = toRefs(props);

const isMediaReaderBook = (book: BookResource | null | undefined): boolean =>
  book?.mediaType === "audio" || book?.mediaType === "video";
</script>

<template>
  <section
    v-if="bookSourceSwitch.bookSourceSwitchOpen"
    class="overlay-backdrop book-source-switch-backdrop"
    @click.self="bookSourceSwitch.closeBookSourceSwitch"
    @keydown.esc.stop.prevent="bookSourceSwitch.closeBookSourceSwitch"
  >
    <article class="book-source-switch-panel" role="dialog" aria-modal="true" aria-labelledby="book-source-switch-title">
      <header class="book-source-switch-heading">
        <div>
          <h2 id="book-source-switch-title">更换源</h2>
          <small>{{ bookSourceSwitch.bookSourceSwitchBook?.title }}</small>
        </div>
        <button type="button" class="book-source-switch-close" data-back-action aria-label="关闭换源" @click="bookSourceSwitch.closeBookSourceSwitch"><PrototypeIcon name="close" /></button>
      </header>
      <div class="book-source-switch-body">
      <p class="book-source-switch-description">整书换源会重新获取目录，并按章节名称匹配阅读位置和书签。</p>
      <p
        v-if="bookSourceSwitch.bookSourceChapterTargetId && !bookSourceSwitch.bookSourceCurrentChapter"
        class="book-source-switch-description"
      >
        所选章节“{{ bookSourceSwitch.bookSourceChapterTargetTitle }}”已不在当前目录中。请关闭窗口后重新选择章节。
      </p>
      <p
        v-else-if="bookSourceSwitch.bookSourceCurrentChapter && !isMediaReaderBook(bookSourceSwitch.bookSourceSwitchBook)"
        class="book-source-switch-description"
      >
        {{ bookSourceSwitch.bookSourceChapterTargetId ? '目标章节' : '当前章节' }}：{{ bookSourceSwitch.bookSourceChapterTargetTitle || bookSourceSwitch.bookSourceCurrentChapter.title }}。你也可以从其他源只替换本章正文。
      </p>
      <p v-else-if="!isMediaReaderBook(bookSourceSwitch.bookSourceSwitchBook)" class="book-source-switch-description">
        保存一章阅读进度后，可从候选源只替换该章正文。
      </p>
      <p v-else class="book-source-switch-description">音频和视频书籍只支持整书切换源，不支持替换单章内容。</p>

      <div class="book-source-switch-toolbar">
        <span>{{ bookSourceSwitch.bookSourceSearchStatus || '可用源' }}<small>{{ bookSourceCandidates.length }} 个可用源</small></span>
        <label class="book-source-search-keyword">
          <span>搜索词</span>
          <input
            v-model="bookSourceSwitch.bookSourceSearchKeyword"
            autocomplete="off"
            :disabled="bookSourceSwitch.bookSourceSearchBusy || bookSourceSwitch.bookSourceSwitchBusy"
            placeholder="留空按原书名搜索"
          />
        </label>
        <button
          type="button" class="button secondary small"
          :disabled="bookSourceSwitch.bookSourceSearchBusy || bookSourceSwitch.bookSourceSwitchBusy || !bookSourceCandidates.length || shelfBatchRecoveryRequired"
          @click="bookSourceSwitch.startBookSourceCandidateSearch"
        >
          {{ bookSourceSwitch.bookSourceSearchBusy ? '正在搜索…' : '重新搜索' }}
        </button>
      </div>

      <p v-if="bookSourceSwitch.chapterSourceError" class="source-identity-confirmation">{{ bookSourceSwitch.chapterSourceError }}</p>
      <div v-if="bookSourceSwitch.bookSourceSearchBusy" class="source-switch-state">
        <span class="loader-ring"></span>
        <strong>{{ bookSourceSwitch.bookSourceSearchStatus || '正在寻找匹配书籍' }}</strong>
        <p>关闭窗口会停止候选源搜索。</p>
      </div>
      <div v-else-if="bookSourceSwitch.bookSourceSwitchError" class="source-switch-state source-switch-error">
        <strong>暂时无法完成</strong>
        <p>{{ bookSourceSwitch.bookSourceSwitchError }}</p>
        <button
          class="button secondary"
          :disabled="!bookSourceCandidates.length || shelfBatchRecoveryRequired"
          @click="bookSourceSwitch.startBookSourceCandidateSearch"
        >重新搜索</button>
      </div>
      <div v-else-if="bookSourceSwitch.bookSourceCandidateResults.length" class="book-source-candidate-list">
        <article
          v-for="candidate in bookSourceSwitch.bookSourceCandidateResults"
          :key="candidate.resultId"
          class="book-source-candidate"
          :class="{ selected: bookSourceSwitch.selectedBookSourceCandidate?.resultId === candidate.resultId }"
        >
          <img v-if="candidate.coverSrc" :src="candidate.coverSrc" loading="lazy" decoding="async" alt="" />
          <div v-else class="book-source-candidate-cover">{{ candidate.title.slice(0, 1) }}</div>
          <div class="book-source-candidate-copy">
            <span class="source-label">{{ candidate.sourceName }}</span>
            <strong>{{ candidate.title }}</strong>
            <small>{{ candidate.author || '作者未知' }}</small>
            <p v-if="candidate.requiresIdentityConfirmation" class="source-candidate-identity-warning">作者信息缺失，需要你确认书名对应同一本书。</p>
            <p v-else-if="candidate.latestChapter">最新章节：{{ candidate.latestChapter }}</p>
            <p v-if="bookSourceSwitch.chapterSourceIdentityConfirmResultId === candidate.resultId" class="source-candidate-identity-warning">
              请确认这个书名与当前书籍对应，再替换本章正文。
            </p>
            <button
              v-if="bookSourceSwitch.bookSourceCurrentChapter && !isMediaReaderBook(bookSourceSwitch.bookSourceSwitchBook) && candidate.sourceId !== bookSourceSwitch.bookSourceSwitchBook?.sourceId && bookSourceSwitch.chapterSourceIdentityConfirmResultId !== candidate.resultId"
              class="button secondary small"
              :disabled="bookSourceSwitch.bookSourceSwitchBusy || bookSourceSwitch.bookSourceSearchBusy || shelfBatchRecoveryRequired"
              @click="bookSourceSwitch.replaceCurrentChapterFromCandidate(candidate)"
            >
              {{ bookSourceSwitch.chapterSourceBusyResultId === candidate.resultId ? '正在替换本章…' : '替换本章正文' }}
            </button>
            <template
              v-else-if="bookSourceSwitch.bookSourceCurrentChapter && !isMediaReaderBook(bookSourceSwitch.bookSourceSwitchBook) && bookSourceSwitch.chapterSourceIdentityConfirmResultId === candidate.resultId"
            >
              <button
                class="button primary small"
                :disabled="bookSourceSwitch.bookSourceSwitchBusy || bookSourceSwitch.bookSourceSearchBusy || shelfBatchRecoveryRequired"
                @click="bookSourceSwitch.replaceCurrentChapterFromCandidate(candidate, true)"
              >
                {{ bookSourceSwitch.chapterSourceBusyResultId === candidate.resultId ? '正在替换本章…' : '确认并替换本章正文' }}
              </button>
              <button
                type="button"
                class="text-button"
                :disabled="bookSourceSwitch.bookSourceSwitchBusy"
                @click="bookSourceSwitch.chapterSourceIdentityConfirmResultId = null"
              >取消</button>
            </template>
          </div>
          <button
            type="button" class="button secondary small"
            :disabled="bookSourceSwitch.bookSourceSwitchBusy || candidate.sourceId === bookSourceSwitch.bookSourceSwitchBook?.sourceId"
            @click="bookSourceSwitch.selectBookSourceCandidate(candidate)"
          >
            {{ candidate.sourceId === bookSourceSwitch.bookSourceSwitchBook?.sourceId ? '当前' : bookSourceSwitch.selectedBookSourceCandidate?.resultId === candidate.resultId ? '已选择' : '选择' }}
          </button>
        </article>
      </div>
      <div v-else-if="bookSourceSwitch.bookSourceCandidateErrors.length" class="book-source-candidate-errors">
        <strong>部分源没有完成搜索</strong>
        <p v-for="(error, index) in bookSourceSwitch.bookSourceCandidateErrors" :key="`${error.sourceId ?? 'source'}-${index}`">
          {{ error.sourceId ? sourceName(error.sourceId) : '源' }}：{{ error.message }}
        </p>
      </div>
      <div v-else-if="bookSourceSwitch.bookSourceCandidateTaskId && !bookSourceSwitch.bookSourceSearchBusy" class="source-switch-state">
        <strong>没有找到匹配的其他源</strong>
        <p>可以稍后重试，或先检查是否启用了其他可用源。</p>
        <button
          class="button secondary"
          :disabled="!bookSourceCandidates.length || shelfBatchRecoveryRequired"
          @click="bookSourceSwitch.startBookSourceCandidateSearch"
        >重新搜索</button>
      </div>
      <div v-else class="source-switch-state">
        <strong>准备搜索候选源</strong>
        <p>{{ bookSourceCandidates.length ? '正在查找名称和作者相符的书籍。' : '请先启用其他可用源。' }}</p>
      </div>
      <div v-if="bookSourceSwitch.bookSourceCandidateErrors.length && bookSourceSwitch.bookSourceCandidateResults.length" class="book-source-candidate-errors">
        <strong>部分源未能完成</strong>
        <p v-for="(error, index) in bookSourceSwitch.bookSourceCandidateErrors" :key="`partial-${error.sourceId ?? 'source'}-${index}`">
          {{ error.sourceId ? sourceName(error.sourceId) : '源' }}：{{ error.message }}
        </p>
      </div>
      </div>
      <footer class="book-source-switch-footer">
        <button type="button" class="button secondary small" @click="bookSourceSwitch.closeBookSourceSwitch">关闭</button>
        <button
          type="button" class="button primary small"
          :disabled="!bookSourceSwitch.selectedBookSourceCandidate || bookSourceSwitch.bookSourceSwitchBusy || shelfBatchRecoveryRequired"
          @click="bookSourceSwitch.bookSourceSwitchConfirmOpen = true"
        >整书换源</button>
      </footer>
    </article>
  </section>

  <section
    v-if="bookSourceSwitch.bookSourceSwitchConfirmOpen && bookSourceSwitch.selectedBookSourceCandidate"
    class="modal-backdrop book-source-change-confirm-backdrop"
    @click.self="!bookSourceSwitch.bookSourceSwitchBusy && (bookSourceSwitch.bookSourceSwitchConfirmOpen = false)"
    @keydown.esc.stop.prevent="!bookSourceSwitch.bookSourceSwitchBusy && (bookSourceSwitch.bookSourceSwitchConfirmOpen = false)"
  >
    <article class="app-modal book-source-change-confirm" role="dialog" aria-modal="true" aria-labelledby="book-source-change-title">
      <h2 id="book-source-change-title">确认更换源？</h2>
      <p>将《{{ bookSourceSwitch.bookSourceSwitchBook?.title }}》切换到“{{ bookSourceSwitch.selectedBookSourceCandidate.sourceName }}”。阅读位置会按新目录更新；无法匹配的书签仍会保留，并标记为旧章节。</p>
      <p v-if="bookSourceSwitch.bookSourceNeedsIdentityConfirmation" class="source-identity-confirmation">
        {{ bookSourceSwitch.bookSourceChangeError || '这个候选源缺少可核实的作者信息。请确认书名对应的是同一本书。' }}
      </p>
      <p v-else-if="bookSourceSwitch.bookSourceChangeError" class="source-identity-confirmation">{{ bookSourceSwitch.bookSourceChangeError }}</p>
      <div class="modal-actions">
        <button type="button" class="button secondary small" :disabled="bookSourceSwitch.bookSourceSwitchBusy" data-back-action @click="bookSourceSwitch.bookSourceSwitchConfirmOpen = false">返回</button>
        <button
          type="button" class="button primary small"
          :disabled="bookSourceSwitch.bookSourceSwitchBusy || shelfBatchRecoveryRequired"
          @click="bookSourceSwitch.confirmBookSourceChange"
        >
          {{ bookSourceSwitch.bookSourceSwitchBusy ? '正在更换…' : bookSourceSwitch.bookSourceNeedsIdentityConfirmation ? '确认书名正确并更换' : '确认更换' }}
        </button>
      </div>
    </article>
  </section>
</template>

<style scoped>
.overlay-backdrop {
  position: fixed;
  z-index: 60;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: calc(16px + var(--safe-top)) 16px calc(16px + var(--safe-bottom));
  pointer-events: auto;
  background: rgb(25 27 35 / 36%);
  backdrop-filter: blur(3px);
}

.book-source-change-confirm-backdrop {
  z-index: 61;
  pointer-events: auto;
}

.book-source-switch-panel {
  display: flex;
  flex-direction: column;
  width: min(720px, 100%);
  min-width: 0;
  max-height: min(760px, calc(100dvh - var(--safe-top) - var(--safe-bottom) - 32px));
  overflow: hidden;
  border: 1px solid var(--app-line);
  border-radius: 14px;
  color: var(--app-text);
  background: var(--app-surface);
  box-shadow: var(--app-shadow);
  font-size: var(--ui-font-body);
}
.book-source-switch-heading {
  display: flex;
  flex: none;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 16px 20px;
  border-bottom: 1px solid var(--app-line);
}
.book-source-switch-heading > div { display: grid; min-width: 0; gap: 4px; }
.book-source-switch-heading h2 { margin: 0; font-size: var(--ui-font-heading); }
.book-source-switch-heading small { overflow: hidden; color: var(--app-muted); font-size: var(--ui-font-caption); text-overflow: ellipsis; white-space: nowrap; }
.book-source-switch-close {
  display: grid; width: 32px; height: 32px; flex: none; place-items: center;
  padding: 0; border: 0; border-radius: 8px;
  color: var(--app-muted); background: transparent; cursor: pointer;
}
.book-source-switch-close:hover { color: var(--app-text); background: var(--app-panel-soft); }
.book-source-switch-body {
  flex: 1;
  min-height: 0;
  overflow-x: hidden;
  overflow-y: auto;
  overscroll-behavior: contain;
  padding: 14px 20px;
}
.book-source-switch-description { margin: 0 0 8px; color: var(--app-muted); font-size: var(--ui-font-caption); line-height: 1.6; overflow-wrap: anywhere; }
.book-source-switch-toolbar { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; margin: 14px 0; }
.book-source-switch-toolbar > span { display: grid; flex: 1 0 100%; gap: 3px; font-size: var(--ui-font-caption); }
.book-source-switch-toolbar > span small { color: var(--app-muted); }
.book-source-search-keyword { display: flex; flex: 1; min-width: 0; align-items: center; gap: 8px; }
.book-source-search-keyword > span { flex: none; font-size: var(--ui-font-caption); color: var(--app-muted); }
.book-source-search-keyword input {
  flex: 1; width: 100%; min-width: 0; min-height: 32px; padding: 0 10px;
  border: 1px solid var(--app-line); border-radius: 8px;
  color: var(--app-text); background: var(--app-panel-softer); font-size: var(--ui-font-body);
}
.book-source-candidate-list { display: grid; gap: 8px; }
.book-source-candidate {
  display: grid;
  grid-template-columns: 48px minmax(0, 1fr) auto;
  align-items: start;
  gap: 10px;
  padding: 12px;
  border: 1px solid var(--app-line);
  border-radius: 10px;
}
.book-source-candidate.selected { border-color: var(--app-accent); background: var(--app-accent-soft); }
.book-source-candidate > img,
.book-source-candidate-cover { display: block; width: 48px; height: 64px; border-radius: 6px; object-fit: cover; }
.book-source-candidate-cover { display: grid; place-items: center; color: var(--app-accent-ink); background: var(--app-panel-soft); font-size: var(--ui-font-heading); }
.book-source-candidate-copy { display: grid; grid-template-columns: minmax(0, 1fr); min-width: 0; gap: 4px; }
.book-source-candidate-copy > strong { overflow: hidden; font-size: var(--ui-font-body); text-overflow: ellipsis; white-space: nowrap; }
.book-source-candidate-copy > small,
.book-source-candidate-copy > .source-label { overflow: hidden; color: var(--app-muted); font-size: var(--ui-font-caption); text-overflow: ellipsis; white-space: nowrap; }
.book-source-candidate-copy p { margin: 0; color: var(--app-muted); font-size: var(--ui-font-caption); line-height: 1.5; overflow-wrap: anywhere; }
.book-source-candidate-copy > .button { justify-self: start; max-width: 100%; margin-top: 4px; padding: 4px 10px; white-space: normal; font-size: var(--ui-font-caption); }
.book-source-candidate > .button { align-self: center; }
.source-switch-state { display: grid; justify-items: center; gap: 8px; min-height: 120px; padding: 20px 0; text-align: center; }
.source-switch-state strong { font-size: var(--ui-font-body); }
.source-switch-state p { margin: 0; color: var(--app-muted); font-size: var(--ui-font-caption); line-height: 1.6; overflow-wrap: anywhere; }
.source-switch-error strong,
.book-source-candidate-copy .source-candidate-identity-warning,
.source-identity-confirmation { color: var(--app-warning); }
.source-identity-confirmation { margin: 8px 0; font-size: var(--ui-font-caption); line-height: 1.6; overflow-wrap: anywhere; }
.book-source-candidate-errors { margin-top: 12px; color: var(--app-muted); font-size: var(--ui-font-caption); overflow-wrap: anywhere; }
.book-source-candidate-errors p { margin: 6px 0; line-height: 1.5; }
.book-source-switch-footer { display: flex; flex: none; align-items: center; justify-content: flex-end; gap: 8px; padding: 12px 20px; border-top: 1px solid var(--app-line); }
.book-source-change-confirm { width: min(420px, 100%); color: var(--app-text); }
.book-source-change-confirm h2 { margin: 0 0 12px; font-size: var(--ui-font-heading); }
.book-source-change-confirm p { font-size: var(--ui-font-body); line-height: 1.6; overflow-wrap: anywhere; }
.book-source-change-confirm .modal-actions { flex-wrap: wrap; }
.book-source-switch-panel button:focus-visible,
.book-source-change-confirm button:focus-visible { outline: 2px solid var(--app-accent); outline-offset: 2px; }
.book-source-search-keyword input:focus-visible { outline: 2px solid var(--app-accent); outline-offset: 1px; }

@media (max-width: 530px) {
  .overlay-backdrop { padding-inline: 12px; }
  .book-source-switch-heading { padding: 14px; }
  .book-source-switch-body { padding: 12px; }
  .book-source-switch-footer { padding: 12px; }
  .book-source-candidate { grid-template-columns: 40px minmax(0, 1fr) auto; gap: 8px; padding: 10px; }
  .book-source-candidate > img,
  .book-source-candidate-cover { width: 40px; height: 54px; }
  .book-source-search-keyword > span { display: none; }
  .book-source-switch-footer > .button.primary { flex: 1; }
}
</style>

<script setup lang="ts">
import { toRefs } from "vue";
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
  >
    <article class="book-source-switch-panel">
      <header class="book-source-switch-heading">
        <div>
          <p class="eyebrow">更换书源</p>
          <h2 id="book-source-switch-title">为《{{ bookSourceSwitch.bookSourceSwitchBook?.title }}》寻找其他来源</h2>
        </div>
        <button class="detail-close" @click="bookSourceSwitch.closeBookSourceSwitch">×</button>
      </header>
      <p class="book-source-switch-description">搜索结果只显示匹配的书籍。整书换源会重新获取目录，并尽量按章节名称保留阅读位置和书签。</p>
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
        {{ bookSourceSwitch.bookSourceChapterTargetId ? '目标章节' : '当前章节' }}：{{ bookSourceSwitch.bookSourceChapterTargetTitle || bookSourceSwitch.bookSourceCurrentChapter.title }}。你也可以从其他书源只替换本章正文。
      </p>
      <p v-else-if="!isMediaReaderBook(bookSourceSwitch.bookSourceSwitchBook)" class="book-source-switch-description">
        保存一章阅读进度后，可从候选书源只替换该章正文。
      </p>
      <p v-else class="book-source-switch-description">音频和视频书籍只支持整书切换书源，不支持替换单章内容。</p>

      <div class="book-source-switch-toolbar">
        <span>{{ bookSourceSwitch.bookSourceSearchStatus || '可用书源' }}<small>{{ bookSourceCandidates.length }} 个已启用来源</small></span>
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
          class="button secondary"
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
        <p>搜索可能需要一些时间，你可以关闭窗口稍后再查看任务。</p>
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
          <img v-if="candidate.coverSrc" :src="candidate.coverSrc" loading="lazy" />
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
            class="button secondary"
            :disabled="bookSourceSwitch.bookSourceSwitchBusy || candidate.sourceId === bookSourceSwitch.bookSourceSwitchBook?.sourceId"
            @click="bookSourceSwitch.selectBookSourceCandidate(candidate)"
          >
            {{ candidate.sourceId === bookSourceSwitch.bookSourceSwitchBook?.sourceId ? '当前' : bookSourceSwitch.selectedBookSourceCandidate?.resultId === candidate.resultId ? '已选择' : '选择' }}
          </button>
        </article>
      </div>
      <div v-else-if="bookSourceSwitch.bookSourceCandidateErrors.length" class="book-source-candidate-errors">
        <strong>部分书源没有完成搜索</strong>
        <p v-for="(error, index) in bookSourceSwitch.bookSourceCandidateErrors" :key="`${error.sourceId ?? 'source'}-${index}`">
          {{ error.sourceId ? sourceName(error.sourceId) : '书源' }}：{{ error.message }}
        </p>
      </div>
      <div v-else-if="bookSourceSwitch.bookSourceCandidateTaskId && !bookSourceSwitch.bookSourceSearchBusy" class="source-switch-state">
        <strong>没有找到匹配的其他书源</strong>
        <p>可以稍后重试，或先检查是否启用了其他可用书源。</p>
        <button
          class="button secondary"
          :disabled="!bookSourceCandidates.length || shelfBatchRecoveryRequired"
          @click="bookSourceSwitch.startBookSourceCandidateSearch"
        >重新搜索</button>
      </div>
      <div v-else class="source-switch-state">
        <strong>准备搜索候选书源</strong>
        <p>{{ bookSourceCandidates.length ? '正在查找名称和作者相符的书籍。' : '请先启用其他可用书源。' }}</p>
      </div>
      <div v-if="bookSourceSwitch.bookSourceCandidateErrors.length && bookSourceSwitch.bookSourceCandidateResults.length" class="book-source-candidate-errors">
        <strong>部分书源未能完成</strong>
        <p v-for="(error, index) in bookSourceSwitch.bookSourceCandidateErrors" :key="`partial-${error.sourceId ?? 'source'}-${index}`">
          {{ error.sourceId ? sourceName(error.sourceId) : '书源' }}：{{ error.message }}
        </p>
      </div>
      <footer class="book-source-switch-footer">
        <button class="button secondary" @click="bookSourceSwitch.closeBookSourceSwitch">关闭</button>
        <button
          class="button primary"
          :disabled="!bookSourceSwitch.selectedBookSourceCandidate || bookSourceSwitch.bookSourceSwitchBusy || shelfBatchRecoveryRequired"
          @click="bookSourceSwitch.bookSourceSwitchConfirmOpen = true"
        >整书更换为所选书源</button>
      </footer>
    </article>
  </section>

  <section
    v-if="bookSourceSwitch.bookSourceSwitchConfirmOpen && bookSourceSwitch.selectedBookSourceCandidate"
    class="modal-backdrop book-source-change-confirm-backdrop"
    @click.self="!bookSourceSwitch.bookSourceSwitchBusy && (bookSourceSwitch.bookSourceSwitchConfirmOpen = false)"
  >
    <article class="app-modal confirm-modal">
      <div class="confirm-symbol">⇄</div>
      <h2>确认更换书源？</h2>
      <p>将《{{ bookSourceSwitch.bookSourceSwitchBook?.title }}》切换到“{{ bookSourceSwitch.selectedBookSourceCandidate.sourceName }}”。阅读位置会按新目录更新；无法匹配的书签仍会保留，并标记为旧章节。</p>
      <p v-if="bookSourceSwitch.bookSourceNeedsIdentityConfirmation" class="source-identity-confirmation">
        {{ bookSourceSwitch.bookSourceChangeError || '这个候选书源缺少可核实的作者信息。请确认书名对应的是同一本书。' }}
      </p>
      <p v-else-if="bookSourceSwitch.bookSourceChangeError" class="source-identity-confirmation">{{ bookSourceSwitch.bookSourceChangeError }}</p>
      <div class="modal-actions">
        <button class="button secondary" :disabled="bookSourceSwitch.bookSourceSwitchBusy" @click="bookSourceSwitch.bookSourceSwitchConfirmOpen = false">返回候选列表</button>
        <button
          class="button primary"
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
  z-index: 50;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 16px;
  background: rgb(25 27 35 / 36%);
  backdrop-filter: blur(3px);
}

.book-source-change-confirm-backdrop {
  z-index: 51;
}
</style>

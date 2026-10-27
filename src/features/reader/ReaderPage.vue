<script lang="ts">
import { Range as KRange, Checkbox as KCheckbox, Button as KButton, Glass as KGlass, Toolbar as KToolbar, ToolbarPane as KToolbarPane, List as KList, ListItem as KListItem, Radio as KRadio } from "konsta/vue";
import { computed, defineAsyncComponent, defineComponent, nextTick, onUnmounted, proxyRefs, ref, watch, type PropType } from "vue";
import type { ReaderContextFeatures } from "./readerContext";
import type { ChapterResource } from "../../api/types";
import type { BookSourceSwitchContract } from "../sources/sourceContext";
import SubpageHeader from "../../ui/SubpageHeader.vue";
import BackButton from "../../ui/BackButton.vue";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import BookSourceSwitchView from "../sources/BookSourceSwitchView.vue";
import BookCatalogPanel from "../books/BookCatalogPanel.vue";
import { settings } from "../settings/settingsStore";
import { isReadingMode, isVerticalReadingMode, readingModes } from "../settings/readerModes";
import { appThemes, readingThemes, resolvedReadingTheme, setReaderTheme } from "../settings/themeState";
import { httpTtsConfigs } from "../settings/configurationStores";
import { readingBook } from "./readerBookState";

export default defineComponent({
  props: {
    readerFeatures: { type: Object as PropType<ReaderContextFeatures>, required: true },
    closeReader: { type: Function as PropType<(discardProgress?: boolean) => Promise<void>>, required: true },
    bookSourceSwitch: { type: Object as PropType<BookSourceSwitchContract>, required: true },
    shelfBatchRecoveryRequired: { type: Boolean, required: true },
  },
  components: { SubpageHeader, KRange, KCheckbox, KButton, KGlass, KToolbar, KToolbarPane, KList, KListItem, KRadio, BackButton, PrototypeIcon, BookSourceSwitchView, BookCatalogPanel,
    TtsSettingsPage: defineAsyncComponent(() => import("../settings/TtsSettingsPage.vue")) },
  setup(props) {
    const context = {
      ...props.readerFeatures,
      ...props.readerFeatures.readerChapterLoading,
      openBookSourceSwitch: openReaderSources,
      closeReader: props.closeReader,
    };
    const comicBrightness = ref(100);
    async function openReaderSources(_fromReader = true): Promise<void> {
      context.readerDirectoryOpen.value = false;
      await props.bookSourceSwitch.openBookSourceSwitch(true);
    }
    async function replaceReaderChapterSource(chapter: ChapterResource): Promise<void> {
      context.readerDirectoryOpen.value = false;
      await props.bookSourceSwitch.openBookSourceSwitch(true, { id: chapter.id, title: chapter.title });
    }
    const horizontalReaderEdges = computed(() => context.readingImagePage.value || context.readingPdfPage.value
      || (!context.readingMediaChapter.value && !isVerticalReadingMode(settings.value.reader.readingMode)));
    const audioToolPanel = ref<"speed" | "sleep" | null>(null);
    function openAudioToolPanel(panel: "speed" | "sleep"): void {
      audioToolPanel.value = panel;
      context.openReaderControls("tts");
    }
    function openAudioAdvanced(): void {
      audioToolPanel.value = null;
      context.openReaderControls("tts");
    }
    function changeReaderMode(event: Event): void {
      const mode = (event.target as HTMLSelectElement).value;
      if (isReadingMode(mode)) settings.value.reader.readingMode = mode;
    }
    watch(() => context.readerControlsOpen.value, open => {
      if (!open) audioToolPanel.value = null;
    });
    const comicReadMode = ref<"vertical" | "paged">("vertical");
    const comicPanel = ref<"mode" | "fit" | "brightness" | null>(null);
    const comicPageIndex = ref(0);
    const comicPageCount = ref(0);
    let comicFrame: number | null = null;
    function updateComicCounter(): void {
      comicFrame = null;
      const viewport = context.readerViewport.value;
      if (!viewport || !context.readingImagePage.value) return;
      const images = [...viewport.querySelectorAll<HTMLImageElement>("img")]
        .filter(image => image.getBoundingClientRect().height > 0);
      comicPageCount.value = images.length;
      if (!images.length) { comicPageIndex.value = 0; return; }
      const bounds = viewport.getBoundingClientRect();
      const paged = comicReadMode.value === "paged";
      const reference = paged ? bounds.left + viewport.clientWidth / 2 : bounds.top + viewport.clientHeight * 0.45;
      const next = images.findIndex(image => {
        const rect = image.getBoundingClientRect();
        return paged ? rect.right > reference : rect.bottom > reference;
      });
      comicPageIndex.value = next < 0 ? images.length : next + 1;
    }
    function requestComicCounter(): void {
      if (comicFrame !== null) return;
      comicFrame = window.requestAnimationFrame(updateComicCounter);
    }
    watch(comicReadMode, mode => {
      const pageIndex = Math.max(0, comicPageIndex.value - 1);
      void nextTick(() => {
        const viewport = context.readerViewport.value;
        const image = viewport?.querySelectorAll<HTMLImageElement>("img")[pageIndex];
        if (viewport && image) {
          const bounds = viewport.getBoundingClientRect();
          const rect = image.getBoundingClientRect();
          const left = viewport.scrollLeft + rect.left - bounds.left;
          const top = viewport.scrollTop + rect.top - bounds.top;
          viewport.scrollTo({ left: mode === "paged" ? left : 0, top: mode === "vertical" ? top : 0, behavior: "instant" });
        }
        requestComicCounter();
      });
    });
    onUnmounted(() => { if (comicFrame !== null) window.cancelAnimationFrame(comicFrame); });
    function nextComicImage(direction: -1 | 1): void {
      if (context.readerGesturesBlocked.value) return;
      const scroller = context.readerViewport.value;
      if (!scroller) return;
      if (comicReadMode.value === "paged") {
        const images = [...scroller.querySelectorAll<HTMLImageElement>("img")]
          .filter(image => image.getBoundingClientRect().height > 0);
        if (!images.length) return;
        const bounds = scroller.getBoundingClientRect();
        const center = bounds.left + scroller.clientWidth / 2;
        let currentIndex = 0;
        let nearestDistance = Number.POSITIVE_INFINITY;
        images.forEach((image, index) => {
          const rect = image.getBoundingClientRect();
          const distance = Math.abs((rect.left + rect.right) / 2 - center);
          if (distance < nearestDistance) {
            nearestDistance = distance;
            currentIndex = index;
          }
        });
        const nextIndex = currentIndex + direction;
        if (nextIndex < 0 || nextIndex >= images.length) {
          void context.changeChapter(direction);
          return;
        }
        const target = images[nextIndex]!.getBoundingClientRect();
        const left = scroller.scrollLeft + target.left - bounds.left;
        scroller.scrollTo({ left: Math.max(0, Math.min(scroller.scrollWidth - scroller.clientWidth, left)), top: 0, behavior: "smooth" });
        return;
      }
      const max = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
      const offset = Math.max(0, scroller.scrollTop);
      const step = Math.max(120, scroller.clientHeight * 0.86);
      const goal = Math.max(0, Math.min(max, offset + direction * step));
      if (Math.abs(goal - offset) > 1) scroller.scrollTo({ top: goal, behavior: "smooth" });
      else void context.changeChapter(direction);
    }
    function handleComicTapAt(x: number): void {
      if (context.readerGesturesBlocked.value) return;
      if (x < window.innerWidth / 3) nextComicImage(-1);
      else if (x > window.innerWidth * 2 / 3) nextComicImage(1);
      else context.readerMenuOpen.value = true;
    }
    function turnReaderEdge(direction: -1 | 1): void {
      if (context.readerGesturesBlocked.value) return;
      if (context.readingImagePage.value) nextComicImage(direction);
      else context.turnPage(direction);
    }
    function openComicPanel(panel: "mode" | "fit" | "brightness"): void {
      comicPanel.value = panel;
      context.openReaderControls("appearance");
    }
    function onComicTouchStart(event: TouchEvent): void {
      comicTouchStart = null;
      if (context.readerGesturesBlocked.value || !context.readingImagePage.value || event.touches.length !== 1) return;
      const touch = event.touches.item(0);
      if (touch) comicTouchStart = { x: touch.clientX, y: touch.clientY };
    }
    function onComicTouchEnd(event: TouchEvent): void {
      const start = comicTouchStart;
      comicTouchStart = null;
      if (!start || context.readerGesturesBlocked.value || !context.readingImagePage.value
        || event.touches.length || event.changedTouches.length !== 1) return;
      const touch = event.changedTouches.item(0);
      if (!touch) return;
      const dx = touch.clientX - start.x, dy = touch.clientY - start.y;
      if (Math.abs(dx) >= 12 || Math.abs(dy) >= 12) return;
      event.stopPropagation();
      handleComicTapAt(touch.clientX);
    }
    let comicTouchStart: { x: number; y: number } | null = null;
    const clampValue = (value: number, min: number, max: number): number =>
      Math.min(max, Math.max(min, value));
    return { ...context, ctx: proxyRefs(context), readingBook, settings, readingModes, horizontalReaderEdges, appThemes, readingThemes, resolvedReadingTheme, setReaderTheme, httpTtsConfigs, clampValue, comicBrightness, comicPageIndex, comicPageCount, requestComicCounter, onComicTouchStart, onComicTouchEnd, comicReadMode, comicTapAt: handleComicTapAt, comicPanel, audioToolPanel, openAudioToolPanel, openAudioAdvanced, nextComicImage, turnReaderEdge, openComicPanel, changeReaderMode, replaceReaderChapterSource };
  },
});
</script>

<template>
  <section v-if="ctx.readerVisible" class="reader-shell"
    :data-reader-menu-open="readerMenuOpen"
    :data-reader-mode="ctx.readingPdfPage ? 'pdf' : ctx.readingVideoChapter ? 'video' : ctx.readingMediaChapter ? 'audio' : ctx.readingImagePage ? 'comic' : 'novel'"
    :class="{ 'reader-menu-open': ctx.readingMediaChapter || readerMenuOpen || readerBusy || ctx.readerChapterError, 'reader-chrome-hidden': !ctx.readingMediaChapter && !readerMenuOpen && !readerBusy && !ctx.readerChapterError, 'audio-console-active': ctx.readingMediaChapter && !ctx.readingVideoChapter && !readerBusy && !ctx.readerChapterError && !!ctx.readingContent }"
    @click="onReaderSurfaceClick">
    <SubpageHeader class="reader-topbar" :class="ctx.readingMediaChapter ? 'relative shrink-0' : 'absolute'" :title="ctx.readingMediaChapter && !ctx.readingVideoChapter ? '音频播放' : readingBook?.title"
      :subtitle="!ctx.readingMediaChapter ? currentChapterTitle : undefined"
      :back-label="ctx.canMinimizeReader ? '收起播放器' : '返回内容库'" :back-disabled="readerMediaRetrying"
      @back="ctx.canMinimizeReader ? ctx.minimizeReader() : closeReader()">
      <template #actions><div class="reader-header-actions">
          <span class="read-save-state" :class="progressSaveState">{{ progressSaveState === 'saving' ? '保存中' :
            progressSaveState === 'saved' ? '已保存' : progressSaveState === 'error' ? '保存失败' : '' }}</span>
          <KButton rounded inline clear v-if="!ctx.readingMediaChapter && !ctx.readingPdfPage && !ctx.readingImagePage" type="button"
            class="reader-tool-button size-11 p-0 text-current" aria-label="在本章中查找" @click.stop="ctx.openReaderControls('search')">
            <PrototypeIcon name="search" />
          </KButton>
          <KButton rounded inline clear v-if="!ctx.readingMediaChapter && !ctx.readingPdfPage && readingBook?.canChangeSource" type="button"
            class="reader-tool-button size-11 p-0 text-current" aria-label="切换内容来源" :disabled="readerBusy || bookSourceSwitch.bookSourceSwitchBusy"
            @click.stop="openBookSourceSwitch(true)">
            <PrototypeIcon name="source-switch" />
          </KButton>
          <KButton rounded inline clear type="button" class="reader-tool-button size-11 p-0 text-current" :aria-label="ctx.readingMediaChapter ? '播放设置' : '更多阅读选项'"
            @click.stop="ctx.readingMediaChapter ? openAudioAdvanced() : ctx.openReaderControls(ctx.readingPdfPage ? 'appearance' : 'settings')">
            <PrototypeIcon name="more" />
          </KButton>
        </div></template>
    </SubpageHeader>
    <div class="reader-main" @touchmove="ctx.readerGesturesBlocked && $event.preventDefault()"
      @wheel="ctx.readerGesturesBlocked && $event.preventDefault()">
      <output v-if="ctx.readingImagePage && comicPageCount" class="reader-comic-page-counter">{{ comicPageIndex }} / {{
        comicPageCount }}</output>
      <div v-if="ctx.bookmarkPullDistance > 0" class="reader-bookmark-pull-hint" :class="{ ready: ctx.bookmarkPullReady }" role="status">
        <PrototypeIcon name="bookmark" />
        <span>{{ ctx.bookmarkPullReady ? '松手' : '下拉' }}{{ currentBookmark ? '移除书签' : '添加书签' }}</span>
      </div>
      <span v-if="currentBookmark && !readerMenuOpen && !ctx.readingMediaChapter && !ctx.readingPdfPage && !ctx.readingImagePage"
        class="reader-bookmark-ribbon" aria-label="当前页已有书签"></span>
      <div class="reader-content-frame" :class="{ 'reader-bookmark-pulling': ctx.bookmarkPullDistance > 0 }"
        :style="ctx.readingImagePage ? { filter: `brightness(${comicBrightness}%)` } : ctx.bookmarkPullDistance ? { transform: `translateY(${ctx.bookmarkPullDistance}px)` } : undefined">
        <div v-if="readerBusy && !ctx.readerMediaRetrying" class="reader-loading"><span class="loader-ring"></span>
          <p>{{ readerMediaRetrying ? '正在从源更新缓存…' : '正在打开…' }}</p>
        </div>
        <div v-else-if="ctx.readerChapterError" class="reader-loading reader-load-error">
          <strong>第 {{ (ctx.readingChapterIndex) + 1 }} 章暂时无法显示</strong>
          <p>{{ ctx.readerChapterError }}</p>
          <div class="reader-load-actions">
            <KButton rounded inline tonal small type="button" class="gap-2" :disabled="readerBusy"
              @click.stop="ctx.loadReaderChapter(ctx.readingChapterIndex)">重试缓存章节</KButton>
            <KButton rounded inline tonal small v-if="readingBook?.sourceId && !ctx.readingMediaChapter" type="button" class="gap-2"
              :disabled="readerBusy || readerMediaRetrying || shelfBatchRecoveryRequired"
              @click.stop="ctx.reloadCurrentChapter">从源重新获取</KButton>
            <KButton rounded inline tonal small v-if="readingBook?.canChangeSource" type="button" class="gap-2"
              :disabled="readerBusy || bookSourceSwitch.bookSourceSwitchBusy || shelfBatchRecoveryRequired"
              @click.stop="ctx.openBookSourceSwitch(true)">切换源</KButton>
          </div>
        </div>
        <div v-else-if="!ctx.readingContent && !readerBusy" class="reader-loading reader-load-error">
          <strong>本章正文尚未显示</strong>
          <p>阅读内容没有生成，请重试；仍为空时可以从源重新获取本章。</p>
          <div class="reader-load-actions">
            <KButton rounded inline tonal small type="button" class="gap-2" :disabled="readerBusy"
              @click.stop="ctx.loadReaderChapter(ctx.readingChapterIndex)">重试章节</KButton>
            <KButton rounded inline tonal small v-if="readingBook?.sourceId && !ctx.readingMediaChapter" type="button" class="gap-2"
              :disabled="readerBusy || readerMediaRetrying || shelfBatchRecoveryRequired"
              @click.stop="ctx.reloadCurrentChapter">从源重新获取</KButton>
            <KButton rounded inline tonal small v-if="readingBook?.canChangeSource" type="button" class="gap-2"
              :disabled="readerBusy || bookSourceSwitch.bookSourceSwitchBusy || shelfBatchRecoveryRequired"
              @click.stop="ctx.openBookSourceSwitch(true)">切换源</KButton>
          </div>
        </div>
        <slot v-else :comicReadMode="comicReadMode" :comicCounter="requestComicCounter"
          :comicTouchStart="onComicTouchStart" :comicTouchEnd="onComicTouchEnd"
          :comicTapAt="comicTapAt" />
      </div>
      <section v-if="ctx.readingVideoChapter && !readerBusy && !ctx.readerChapterError && ctx.readingContent"
        class="reader-video-episodes" @click.stop>
        <header>
          <div><strong>{{ currentChapterTitle }}</strong><small>{{ readingBook?.title }} · {{ readerChapterCount }}
              集</small></div>
          <KButton rounded inline clear class="gap-2" type="button" @click="ctx.openReaderDirectory">全部选集
            <PrototypeIcon name="right" />
          </KButton>
        </header>
        <div class="reader-video-options">
          <label v-if="ctx.readerVideoQualityChoices.length > 1" class="reader-video-option">
            <span>清晰度</span>
            <select id="reader-video-quality" :value="ctx.readerVideoQualityIndex"
              :disabled="ctx.isReaderMediaControlBlocked()" @change="onReaderVideoQualityChange">
              <option v-for="(choice, index) in ctx.readerVideoQualityChoices" :key="index" :value="index"
                :disabled="!choice.src || !!choice.unavailableReason">{{ choice.label }}{{ choice.unavailableReason ?
                  ('（' + choice.unavailableReason + '）') : '' }}</option>
            </select>
          </label>
          <label class="reader-video-option">
            <span>倍速</span>
            <select :value="settings.reader.videoPlaybackRate" :disabled="ctx.isReaderMediaControlBlocked()"
              @change="settings.reader.videoPlaybackRate = Number(($event.target as HTMLSelectElement).value)">
              <option v-for="rate in ([0.5, 0.75, 1, 1.25, 1.5, 1.75, 2, 2.25, 2.5, 2.75, 3] as const)" :key="rate" :value="rate">
                {{ rate }}×</option>
            </select>
          </label>
        </div>
        <nav v-if="readingBook?.chapters.length" aria-label="视频选集">
          <KButton rounded :tonal="chapter.index === ctx.readingChapterIndex" :clear="!(chapter.index === ctx.readingChapterIndex)" inline class="gap-2" v-for="chapter in readingBook?.chapters" :key="chapter.id" type="button"
            :class="{ active: chapter.index === ctx.readingChapterIndex }"
            :disabled="readerBusy || shelfBatchRecoveryRequired" :title="chapter.title"
            @click="ctx.selectReaderChapter(chapter.index)">
            {{ chapter.index + 1 }}
          </KButton>
        </nav>
      </section>
      <article
        v-if="ctx.readingMediaChapter && !ctx.readingVideoChapter && !readerBusy && !ctx.readerChapterError && ctx.readingContent"
        class="reader-audio-console" @click.stop>
        <img v-if="readingBook?.coverSrc" class="reader-audio-art" :src="readingBook.coverSrc" alt="音频封面" />
        <div v-else class="reader-audio-art reader-audio-fallback">
          <PrototypeIcon name="audio" />
        </div>
        <div class="reader-audio-body">
          <div class="reader-audio-heading">
            <h2>{{ currentChapterTitle || readingBook?.title }}</h2>
            <p v-if="currentChapterTitle && currentChapterTitle !== readingBook?.title">{{ readingBook?.title }}</p>
            <small v-if="readingBook?.author">{{ readingBook.author }}</small>
          </div>
          <div class="reader-audio-progress">
            <KRange aria-label="播放进度" :min="0" :step="1000" :max="Math.max(1000, ctx.readerMediaDurationMs)"
              :value="ctx.readerMediaTimeMs"
              :disabled="ctx.isReaderMediaControlBlocked() || ctx.readerMediaDurationMs <= 0"
              @change="ctx.seekReaderMedia(Number(($event.target as HTMLInputElement).value) / 1000)"  class="min-w-0 flex-1" />
            <div class="reader-audio-times">
              <span>{{ ctx.formatReaderMediaTime(ctx.readerMediaTimeMs) }}</span>
              <span>{{ ctx.readerMediaDurationMs > 0 ? ctx.formatReaderMediaTime(ctx.readerMediaDurationMs) : '—:—'
                }}</span>
            </div>
          </div>
          <div class="reader-audio-playback">
            <KButton inline clear class="gap-2 rounded-full p-0 size-12" type="button" :aria-label="`后退 ${settings.reader.audioSkipSeconds} 秒`"
              :disabled="ctx.isReaderMediaControlBlocked()"
              @click="ctx.skipReaderMedia(-settings.reader.audioSkipSeconds)">
              <PrototypeIcon name="back15" />
            </KButton>
            <KButton inline type="button" class="reader-audio-main-play gap-2 rounded-full p-0 size-16" :aria-label="ctx.readerMediaPlaying ? '暂停' : '播放'"
              :disabled="ctx.isReaderMediaControlBlocked()" @click="ctx.toggleReaderMediaPlayback()">
              <PrototypeIcon :name="ctx.readerMediaPlaying ? 'pause' : 'play'" />
            </KButton>
            <KButton inline clear class="gap-2 rounded-full p-0 size-12" type="button" :aria-label="`前进 ${settings.reader.audioSkipSeconds} 秒`"
              :disabled="ctx.isReaderMediaControlBlocked()"
              @click="ctx.skipReaderMedia(settings.reader.audioSkipSeconds)">
              <PrototypeIcon name="forward15" />
            </KButton>
          </div>
          <div class="reader-audio-utilities">
            <KButton rounded inline clear class="gap-2 flex-col h-auto min-h-12" type="button" @click="openAudioToolPanel('speed')"><strong>{{ settings.reader.audioPlaybackRate
                }}×</strong><small>倍速</small></KButton>
            <KButton rounded inline clear class="gap-2 flex-col h-auto min-h-12" type="button" @click="openAudioToolPanel('sleep')">
              <PrototypeIcon name="moon" /><small>{{ readerMediaSleepTimerMinutes ? readerMediaSleepTimerMinutes + ' 分钟'
                : '睡眠定时' }}</small>
            </KButton>
            <KButton rounded inline clear class="gap-2 flex-col h-auto min-h-12" type="button" @click="ctx.openReaderDirectory">
              <PrototypeIcon name="list" /><small>节目</small>
            </KButton>
          </div>
        </div>
      </article>
      <div v-if="ctx.readingMediaChapter && ctx.readerMediaError" class="reader-media-error">
        <span>{{ ctx.readerMediaError }}</span>
        <KButton rounded inline tonal small type="button" class="gap-2"
          :disabled="readerBusy || ctx.readerMediaRetrying || shelfBatchRecoveryRequired"
          @click.stop="reloadCurrentChapter">{{ ctx.readerMediaRetrying ? '正在重新加载…' : '重新加载' }}</KButton>
      </div>
      <div v-if="horizontalReaderEdges" class="reader-edge reader-edge-left" @click.stop="turnReaderEdge(-1)"></div>
      <div v-if="horizontalReaderEdges" class="reader-edge reader-edge-right" @click.stop="turnReaderEdge(1)"></div>
      <div v-if="readerMenuOpen && !ctx.readingMediaChapter && !readerBusy && !ctx.readerChapterError"
        class="reader-menu-surface" @click.stop="onReaderSurfaceClick" />
    </div>
    <KToolbar v-if="ctx.readingImagePage" v-show="readerMenuOpen || readerBusy || ctx.readerChapterError" :key="settings.uiStyle + '-comic'"
      component="footer" class="reader-footer reader-comic-dock absolute" tabbar-icons tabbar-labels inner-class="w-full max-w-[640px]">
      <KToolbarPane class="w-full"><nav class="reader-bottom-nav h-full">
        <KButton rounded inline clear type="button" class="reader-nav-action flex-1 flex-col gap-1 h-full min-w-0 px-1 text-current" @click="ctx.openReaderDirectory"><span>
            <PrototypeIcon name="list" />
          </span><small>选话</small></KButton>
        <KButton rounded inline clear type="button" class="reader-nav-action flex-1 flex-col gap-1 h-full min-w-0 px-1 text-current" @click="openComicPanel('mode')"><span>
            <PrototypeIcon name="layout" />
          </span><small>阅读模式</small></KButton>
        <KButton rounded inline clear type="button" class="reader-nav-action flex-1 flex-col gap-1 h-full min-w-0 px-1 text-current" @click="openComicPanel('fit')"><span>
            <PrototypeIcon name="appearance" />
          </span><small>图片适配</small></KButton>
        <KButton rounded inline clear type="button" class="reader-nav-action flex-1 flex-col gap-1 h-full min-w-0 px-1 text-current" @click="openComicPanel('brightness')"><span>
            <PrototypeIcon name="sun" />
          </span><small>亮度</small></KButton>
      </nav></KToolbarPane>
    </KToolbar>
    <KToolbar v-if="ctx.readingPdfPage" v-show="readerMenuOpen || readerBusy || ctx.readerChapterError" :key="settings.uiStyle + '-pdf'"
      component="footer" class="reader-footer reader-pdf-dock absolute" tabbar-icons tabbar-labels inner-class="w-full max-w-[640px]">
      <KToolbarPane class="w-full"><nav class="reader-bottom-nav h-full">
        <KButton rounded inline clear type="button" class="reader-nav-action flex-1 flex-col gap-1 h-full min-w-0 px-1 text-current" @click="ctx.openReaderDirectory"><span>
            <PrototypeIcon name="list" />
          </span><small>页面</small></KButton>
        <KButton rounded inline clear type="button" class="reader-nav-action flex-1 flex-col gap-1 h-full min-w-0 px-1 text-current" @click="ctx.openReaderControls('search')"><span>
            <PrototypeIcon name="search" />
          </span><small>搜索</small></KButton>
        <KButton rounded inline clear type="button" class="reader-nav-action flex-1 flex-col gap-1 h-full min-w-0 px-1 text-current" @click="ctx.openReaderControls('appearance')"><span>
            <PrototypeIcon name="appearance" />
          </span><small>显示</small></KButton>
      </nav></KToolbarPane>
    </KToolbar>
    <KToolbar v-if="!ctx.readingMediaChapter && !ctx.readingImagePage && !ctx.readingPdfPage" v-show="readerMenuOpen || readerBusy || ctx.readerChapterError"
      :key="settings.uiStyle + '-text'" component="footer" class="reader-footer absolute" tabbar-icons tabbar-labels inner-class="w-full max-w-[640px]">
      <KToolbarPane class="w-full"><nav class="reader-bottom-nav h-full">
        <KButton rounded inline clear type="button" class="reader-nav-action flex-1 flex-col gap-1 h-full min-w-0 px-1 text-current" @click="ctx.openReaderDirectory"><span>
            <PrototypeIcon name="list" />
          </span><small>{{ ctx.readingPdfPage ? '页面' : ctx.readingVideoChapter ? '剧集' : ctx.readingMediaChapter ? '节目' :
            ctx.readingImagePage ? '选话' : '目录' }}</small></KButton>
        <KButton rounded :tonal="ctx.readerControlsOpen && readerControlsMode === 'tts'" :clear="!(ctx.readerControlsOpen && readerControlsMode === 'tts')" inline v-if="!ctx.readingPdfPage && !ctx.readingImagePage && !ctx.readingVideoChapter" type="button"
          class="reader-nav-action flex-1 flex-col gap-1 h-full min-w-0 px-1 text-current" :class="{ active: ctx.readerControlsOpen && readerControlsMode === 'tts' }"
          @click="openTtsControls"><span>
            <PrototypeIcon name="tts" />
          </span><small>{{ ttsActive ? ctx.ttsPaused ? '已暂停' : '朗读中' : '朗读' }}</small></KButton>
        <KButton rounded :tonal="ctx.readerControlsOpen && readerControlsMode === 'appearance'" :clear="!(ctx.readerControlsOpen && readerControlsMode === 'appearance')" inline v-if="!ctx.readingMediaChapter" type="button" class="reader-nav-action flex-1 flex-col gap-1 h-full min-w-0 px-1 text-current"
          :class="{ active: ctx.readerControlsOpen && readerControlsMode === 'appearance' }"
          @click="ctx.openReaderControls('appearance')"><span class="reader-appearance-glyph">Aa</span><small>{{
            ctx.readingPdfPage ? '缩放' : ctx.readingImagePage ? '图片适配' : '显示' }}</small></KButton>
        <!-- The header's More action owns advanced settings, as in the prototype. -->
      </nav></KToolbarPane>
    </KToolbar>
    <section v-if="readerTtsSettingsOpen" class="reader-book-page reader-tts-settings" data-back-layer @click.stop>
      <TtsSettingsPage embedded @back="readerTtsSettingsOpen = false" />
    </section>
    <section v-if="readerDirectoryOpen && readingBook" class="reader-book-page" data-back-layer @click.stop
      @keydown.esc.stop.prevent="readerDirectoryOpen = false">
      <header class="reader-book-page-header">
        <BackButton label="返回阅读菜单" @click="readerDirectoryOpen = false" />
        <div><strong>目录</strong><small>{{ readingBook.title }}</small></div>
        <small>{{ readingBook.chapters.length }} 项</small>
      </header>
      <BookCatalogPanel :book="readingBook" :current-index="ctx.readingChapterIndex" :busy="readerBusy" :locked="shelfBatchRecoveryRequired" auto-locate
        :can-download="readingBook.canChangeSource === true && !!readingBook.sourceId"
        :can-replace="readingBook.canChangeSource === true && !readingBook.mediaType"
        @select="ctx.selectReaderChapter" @download="ctx.downloadReaderBook" @replace="replaceReaderChapterSource" />
    </section>
    <div v-if="ctx.readerControlsOpen" class="reader-settings-backdrop" data-back-layer @click.self.stop="ctx.readerControlsOpen = false"
      @keydown.esc.stop.prevent="ctx.readerControlsOpen = false">
    <KGlass component="aside" :highlight="false" class="reader-settings-popover" @click.stop
      :class="{ 'reader-search-popover': readerControlsMode === 'search' }">
      <div class="popover-title"><span>{{ readerControlsMode === 'settings' ? '更多阅读选项' : readerControlsMode === 'tts' ?
        (ctx.readingMediaChapter ? audioToolPanel === 'speed' ? '播放速度' : audioToolPanel === 'sleep' ? '睡眠定时' : '播放设置'
          : '朗读') : readerControlsMode === 'appearance' ? ctx.readingImagePage ? comicPanel === 'mode' ? '阅读模式' :
            comicPanel === 'fit' ? '图片适配' : '亮度' : ctx.readingPdfPage ? 'PDF 显示' : '显示' : '搜索' }}</span><KButton rounded inline clear class="gap-2"
          type="button" data-back-action :aria-label="readerControlsMode === 'tts' && ttsActive ? '收起朗读面板' : '关闭菜单'" @click="ctx.readerControlsOpen = false">
          <PrototypeIcon name="close" />
        </KButton></div>
      <!-- Chapter seek is available in advanced tools; the reading dock keeps only prototype primary actions. -->
      <div
        v-if="readerControlsMode === 'settings' && !ctx.readingMediaChapter && !ctx.readingImagePage && !ctx.readingPdfPage"
        class="reader-settings-progress">
        <div class="reader-chapter-progress">
          <div class="reader-chapter-position">{{ currentChapterTitle }} <span>（{{ ctx.readingChapterIndex + 1 }} / {{
              readerChapterCount }}）</span></div>
          <div v-if="readerChapterCount > 1" class="reader-chapter-seek-row">
            <KRange :min="0" :max="readerChapterCount - 1" :step="1" :value="ctx.readingChapterIndex"
              :disabled="readerBusy || shelfBatchRecoveryRequired" @change="jumpReaderChapter"  class="min-w-0 flex-1" />
          </div>
        </div>

      </div>
      <div v-if="readerControlsMode === 'settings' && !ctx.readingImagePage && !ctx.readingPdfPage"
        class="setting-control compact-setting">
        <div><label for="reader-chapter-jump">跳转章节</label><small>共 {{ readingBook?.chapters?.length ?? 0 }} 章</small>
        </div>
        <input id="reader-chapter-jump" type="number" inputmode="numeric" min="1"
          :max="readingBook?.chapters?.length ?? 0" :value="ctx.readingChapterIndex + 1"
          :disabled="readerBusy || shelfBatchRecoveryRequired" @change="jumpReaderChapter" />
      </div>
      <div
        v-if="readerControlsMode === 'appearance' && !ctx.readingMediaChapter && !ctx.readingPdfPage && !ctx.readingImagePage"
        class="setting-control compact-setting">
        <div><label for="reader-scroll-mode">阅读方式</label><small>分页切换效果或上下连续滚动</small></div>
        <select id="reader-scroll-mode" :value="settings.reader.readingMode ?? 'cover'"
          :disabled="readerBusy" @change="changeReaderMode">
          <option v-for="mode in readingModes" :key="mode.value" :value="mode.value">{{ mode.label }}</option>
        </select>
      </div>
      <div v-if="readerControlsMode === 'tts' && ctx.readingMediaChapter && !ctx.readingVideoChapter && !audioToolPanel"
        class="setting-control reader-audio-rate-control">
        <div><label for="reader-audio-rate">播放速度</label><small>{{ settings.reader.audioPlaybackRate }}×</small></div>
        <KRange input-id="reader-audio-rate" :min="0.5" :max="3" :step="0.25"
          :value="settings.reader.audioPlaybackRate"
          @input="settings.reader.audioPlaybackRate = Number(($event.target as HTMLInputElement).value)"  class="min-w-0 flex-1" />
      </div>
      <div
        v-if="readerControlsMode === 'tts' && ctx.readingMediaChapter && !ctx.readingVideoChapter && audioToolPanel === 'speed'"
        class="reader-audio-choice-grid">
        <KButton rounded :tonal="Math.abs(settings.reader.audioPlaybackRate - rate) < 0.001" :clear="!(Math.abs(settings.reader.audioPlaybackRate - rate) < 0.001)" inline class="gap-2" v-for="rate in ([0.75, 1, 1.25, 1.5, 1.75, 2] as const)" :key="rate" type="button"
          :class="{ selected: Math.abs(settings.reader.audioPlaybackRate - rate) < 0.001 }"
          :disabled="ctx.isReaderMediaControlBlocked()" @click="settings.reader.audioPlaybackRate = rate">{{ rate
          }}×</KButton>
      </div>
      <div v-if="readerControlsMode === 'tts' && ctx.readingMediaChapter && audioToolPanel !== 'speed'"
        class="setting-control compact-setting">
        <div><label for="reader-media-sleep-timer">睡眠定时器</label><small>时间到后暂停并保存位置</small></div>
        <select v-if="audioToolPanel !== 'sleep'" id="reader-media-sleep-timer" :value="readerMediaSleepTimerMinutes"
          :disabled="ctx.isReaderMediaControlBlocked()" @change="onReaderMediaSleepTimerChange">
          <option :value="0">关闭</option>
          <option :value="15">15 分钟</option>
          <option :value="30">30 分钟</option>
          <option :value="60">60 分钟</option>
        </select>
        <KList v-else class="col-span-full my-0!">
          <KListItem v-for="minutes in ([0, 15, 30, 60] as const)" :key="minutes" label
            :title="minutes === 0 ? '关闭' : minutes + ' 分钟'">
            <template #after><KRadio component="span" name="reader-sleep" :checked="readerMediaSleepTimerMinutes === minutes"
              :disabled="ctx.isReaderMediaControlBlocked()" @change="ctx.setReaderMediaSleepTimer(minutes)" /></template>
          </KListItem>
        </KList>
      </div>
      <div v-if="readerControlsMode === 'settings' && !ctx.readingMediaChapter && !ctx.readingPdfPage"
        class="reader-source-tools">
        <KButton rounded inline tonal small v-if="!ctx.readingImagePage && readingBook?.canChangeSource" type="button" class="gap-2"
          :disabled="readerBusy || readerMediaRetrying || bookSourceSwitch.bookSourceSwitchBusy"
          @click.stop="ctx.reloadCurrentChapter">
          <PrototypeIcon name="refresh" />从源重新获取本章
        </KButton>
        <KButton rounded inline tonal small v-if="readingBook?.sourceId" type="button" class="gap-2"
          :disabled="readerBusy || readerBookDownloadBusy || shelfBatchRecoveryRequired"
          @click.stop="ctx.downloadReaderBook">
          <PrototypeIcon name="download" />缓存全部章节
        </KButton>
      </div>
      <div
        v-if="readerControlsMode === 'tts' && !ctx.readingMediaChapter && !ctx.readingPdfPage && !ctx.readingImagePage"
        class="reader-tts-panel">
        <div class="reader-tts-description"><label for="reader-tts-engine">朗读引擎</label><small v-if="ttsNotice">{{
            ttsNotice }}</small><small v-if="ttsActive && ttsChunkProgress">片段 {{
              ttsChunkProgress.current }} / {{ ttsChunkProgress.total }}</small></div>
        <select id="reader-tts-engine" :value="ttsEngineSelection"
          :disabled="ttsActive || readerBusy"
          @change="selectTtsEngine(($event.target as HTMLSelectElement).value)">
          <optgroup label="本地语音">
            <option value="system:">系统默认语音</option>
            <option
              v-if="settings.reader.ttsVoiceURI && !ttsVoices.some((voice) => voice.voiceURI === settings.reader.ttsVoiceURI)"
              :value="`system:${settings.reader.ttsVoiceURI}`">已保存语音不可用（系统默认）</option>
            <option v-for="voice in ttsVoices" :key="voice.voiceURI" :value="`system:${voice.voiceURI}`">{{ voice.name
              }}{{ voice.lang ? `（${voice.lang}）` : '' }}</option>
          </optgroup>
          <optgroup label="网络朗读">
            <option
              v-if="settings.reader.ttsEngine === 'http' && settings.reader.httpTtsConfigId && !httpTtsConfigs.some((item) => item.id === settings.reader.httpTtsConfigId)"
              :value="`http:${settings.reader.httpTtsConfigId}`" disabled>当前网络朗读配置不可用</option>
            <option v-if="!httpTtsConfigs.length" value="http:" disabled>尚未添加网络朗读配置</option>
            <option v-for="item in httpTtsConfigs" :key="item.id" :value="`http:${item.id}`" :disabled="!item.enabled">
              {{ item.name }}{{ item.enabled ? '' : '（已停用）' }}</option>
          </optgroup>
        </select>
        <KButton rounded inline clear v-if="!httpTtsConfigs.some((item) => item.enabled)" type="button" class="gap-2"
          @click="ctx.openHttpTtsSettings">添加网络朗读源</KButton>
        <div class="reader-tts-row"><label for="reader-tts-start-position">朗读起点</label><select
            id="reader-tts-start-position" :value="settings.reader.ttsStartPosition"
            @change="settings.reader.ttsStartPosition = ($event.target as HTMLSelectElement).value === 'chapter' ? 'chapter' : 'current'">
            <option value="current">当前位置</option>
            <option value="chapter">本章开头</option>
          </select></div>
        <div class="reader-tts-row reader-tts-rate"><label for="reader-tts-rate">语速</label><KRange input-id="reader-tts-rate"
            :disabled="ttsActive && settings.reader.ttsEngine === 'system'"
            :min="0.5" :max="settings.reader.ttsEngine === 'system' ? 4 : 5" :step="0.1" :value="settings.reader.ttsRate"
            @input="setTtsRate(Number(($event.target as HTMLInputElement).value))"  class="min-w-0 flex-1" /><small>{{
              settings.reader.ttsRate.toFixed(1) }}×</small></div>
        <label class="reader-tts-continuous"><KCheckbox component="span" :checked="settings.reader.ttsContinueAcrossChapters" :disabled="ttsActive"
            @change="settings.reader.ttsContinueAcrossChapters = ($event.target as HTMLInputElement).checked"  />连续朗读并自动进入下一章</label>
        <div class="reader-tts-selection"><small>{{ ttsSelectedText ? `已选中
            ${ttsSelectedText.length} 个字` : '先在正文中选中文字' }}</small><KButton rounded inline tonal small v-if="!ttsActive"
            type="button" class="gap-2" :disabled="!ttsCanStart || !ttsSelectedText"
            @click="startSelectedTextTts">朗读选中文字</KButton></div>
        <div class="reader-tts-actions">
          <KButton rounded inline tonal small v-if="ttsActive" type="button" class="gap-2"
            @click="ctx.minimizeReader()">收起播放器</KButton>
          <KButton rounded inline tonal small v-if="ttsActive" type="button" class="gap-2"
            :disabled="!ttsChunkProgress" @click="ctx.toggleTtsPaused()">{{ ctx.ttsPaused ? '继续朗读' : '暂停' }}</KButton>
          <KButton rounded inline v-if="!ttsActive" type="button" class="gap-2"
            :disabled="!ttsCanStart" @click="startTts()">开始朗读</KButton>
          <KButton rounded inline tonal small v-if="ttsActive" type="button" class="gap-2"
            @click="stopTts()">停止</KButton>
        </div>
        <p v-if="ttsError">{{ ttsError }}</p>
      </div>
      <div
        v-if="readerControlsMode === 'search' && !ctx.readingMediaChapter && !ctx.readingPdfPage && !ctx.readingImagePage"
        class="setting-control reader-find-control">
        <input id="reader-find-input" v-model="readerFindQuery" type="search" autocomplete="off" placeholder="输入要查找的文字"
          @keydown="handleReaderFindKeydown" />
        <div class="reader-find-actions-row">
          <small>{{ readerFindQuery.trim() ? readerFindMatches.length ? `${readerFindIndex + 1} /
            ${readerFindMatches.length}` : '无匹配' : '' }}</small>
          <div class="reader-find-buttons">
            <KButton rounded inline tonal small type="button" class="gap-2" :disabled="!readerFindMatches.length"
              @click="moveReaderFind(-1)">上一处</KButton>
            <KButton rounded inline tonal small type="button" class="gap-2" :disabled="!readerFindMatches.length"
              @click="moveReaderFind(1)">下一处</KButton>
          </div>
        </div>
      </div>
      <div v-if="readerControlsMode === 'appearance' && ctx.readingPdfPage" class="pdf-scale-control">
        <span>PDF 缩放</span>
        <div>
          <KButton rounded :tonal="ctx.activePdfZoom === 'page-fit'" :clear="!(ctx.activePdfZoom === 'page-fit')" inline class="gap-2" :class="{ selected: ctx.activePdfZoom === 'page-fit' }" @click="setPdfZoom('page-fit')">整页</KButton>
          <KButton rounded :tonal="ctx.activePdfZoom === 'page-width'" :clear="!(ctx.activePdfZoom === 'page-width')" inline class="gap-2" :class="{ selected: ctx.activePdfZoom === 'page-width' }"
            @click="setPdfZoom('page-width')">适合宽度</KButton>
          <KButton rounded :tonal="ctx.activePdfZoom === 'actual-size'" :clear="!(ctx.activePdfZoom === 'actual-size')" inline class="gap-2" :class="{ selected: ctx.activePdfZoom === 'actual-size' }"
            @click="setPdfZoom('actual-size')">原始大小</KButton>
        </div>
      </div>
      <KList v-if="readerControlsMode === 'appearance' && ctx.readingImagePage && comicPanel === 'mode'" class="my-0!">
        <KListItem label title="上下连续" text="条漫和连续滚动阅读">
          <template #after><KRadio component="span" name="comic-mode" :checked="comicReadMode === 'vertical'" @change="comicReadMode = 'vertical'" /></template>
        </KListItem>
        <KListItem label title="左右翻页" text="单页定位，点击两侧或左右滑动切换">
          <template #after><KRadio component="span" name="comic-mode" :checked="comicReadMode === 'paged'" @change="comicReadMode = 'paged'" /></template>
        </KListItem>
      </KList>
      <div v-if="readerControlsMode === 'appearance' && ctx.readingImagePage && comicPanel === 'brightness'"
        class="comic-brightness-control">
        <label for="comic-brightness">亮度</label>
        <KRange input-id="comic-brightness" :value="comicBrightness" @input="comicBrightness = Number(($event.target as HTMLInputElement).value)" :min="50" :max="150" :step="5" class="min-w-0 flex-1" />
        <strong>{{ comicBrightness }}%</strong>
      </div>
      <div v-if="readerControlsMode === 'appearance' && ctx.readingImagePage && comicPanel === 'fit'"
        class="comic-scale-control">
        <span>图片缩放</span>
        <div>
          <KButton rounded :tonal="ctx.comicScaleMode === 'fit-width'" :clear="!(ctx.comicScaleMode === 'fit-width')" inline class="gap-2" :class="{ selected: ctx.comicScaleMode === 'fit-width' }"
            @click="setComicScaleMode('fit-width')">适合宽度</KButton>
          <KButton rounded :tonal="ctx.comicScaleMode === 'actual-size'" :clear="!(ctx.comicScaleMode === 'actual-size')" inline class="gap-2" :class="{ selected: ctx.comicScaleMode === 'actual-size' }"
            @click="setComicScaleMode('actual-size')">原始大小</KButton>
        </div>
      </div>
      <template
        v-if="readerControlsMode === 'appearance' && !ctx.readingMediaChapter && !ctx.readingPdfPage && !ctx.readingImagePage">
        <div class="setting-control compact-setting">
          <div><label>字号</label><small>{{ settings.reader.fontSizePx }} px</small></div>
          <div class="range-control">
            <KButton rounded inline clear class="gap-2" @click="settings.reader.fontSizePx = clampValue(settings.reader.fontSizePx - 1, 12, 36)">−</KButton>
            <KRange :min="12" :max="36" :value="settings.reader.fontSizePx"
              @input="settings.reader.fontSizePx = Number(($event.target as HTMLInputElement).value)"  class="min-w-0 flex-1" />
            <KButton rounded inline clear class="gap-2" @click="settings.reader.fontSizePx = clampValue(settings.reader.fontSizePx + 1, 12, 36)">＋</KButton>
          </div>
        </div>
        <div class="setting-control compact-setting">
          <div><label>行间距</label><small>{{ settings.reader.lineHeight.toFixed(1) }}</small></div>
          <KRange :min="1.2" :max="2.8" :step="0.1" :value="settings.reader.lineHeight"
            @input="settings.reader.lineHeight = Number(($event.target as HTMLInputElement).value)"  class="min-w-0 flex-1" />
        </div>
      </template>
      <div
        v-if="readerControlsMode === 'appearance' && !ctx.readingImagePage && !ctx.readingPdfPage && !ctx.readingMediaChapter"
        class="setting-control compact-setting">
        <div><label>应用主题</label></div>
        <div class="theme-options"><button v-for="theme in appThemes" :key="theme[0]" class="theme-option compact-theme"
            :class="{ selected: settings.theme === theme[0] }" :aria-pressed="settings.theme === theme[0]"
            @click="settings.theme = theme[0]">{{ theme[1] }}</button></div>
      </div>
      <div
        v-if="readerControlsMode === 'appearance' && settings.theme !== 'system' && !ctx.readingImagePage && !ctx.readingPdfPage && !ctx.readingMediaChapter"
        class="setting-control compact-setting">
        <div><label>{{ settings.theme === 'light' ? '白天' : '夜间' }}阅读主题</label></div>
        <div class="theme-options"><button v-for="theme in readingThemes" :key="theme[0]"
            class="theme-option compact-theme" :class="{ selected: resolvedReadingTheme === theme[0] }"
            :aria-pressed="resolvedReadingTheme === theme[0]" @click="setReaderTheme(theme[0])">{{ theme[1] }}</button>
        </div>
      </div>
    </KGlass>
    </div>
  </section>

  <BookSourceSwitchView v-if="ctx.readerVisible && !ctx.readerMinimized" :session="bookSourceSwitch" :locked="shelfBatchRecoveryRequired" />
</template>

<style scoped src="./readerPage.css"></style>

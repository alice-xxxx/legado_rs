<script lang="ts">
import { computed, defineAsyncComponent, defineComponent, onMounted, onUnmounted, proxyRefs, ref, watch, type PropType } from "vue";
import { createAppReaderContext, type ReaderContextFeatures } from "./readerContext";
import type { SourceMetadata } from "../../api/types";
import type { BookSourceSwitchContract } from "../sources/sourceContext";
import BackButton from "../../ui/BackButton.vue";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import BookSourceSwitchSheet from "../sources/BookSourceSwitchSheet.vue";

const PdfReaderPage = defineAsyncComponent(() => import("./PdfReaderPage.vue"));

export default defineComponent({
  props: {
    readerFeatures: { type: Object as PropType<ReaderContextFeatures>, required: true },
    closeReader: { type: Function as PropType<(discardProgress?: boolean) => Promise<void>>, required: true },
    bookSourceSwitch: { type: Object as PropType<BookSourceSwitchContract>, required: true },
    bookSourceCandidates: { type: Array as PropType<SourceMetadata[]>, required: true },
    sourceName: { type: Function as PropType<(id: string) => string>, required: true },
    shelfBatchRecoveryRequired: { type: Boolean, required: true },
  },
  components: { PrototypeIcon, BookSourceSwitchSheet },
  setup(props) {
    const context = createAppReaderContext(props.readerFeatures, {
      bookSourceSwitch: props.bookSourceSwitch,
      closeReader: props.closeReader,
      BackButton,
      PdfReaderPage,
    });
    const comicBrightness = ref(100);
    const pdfFindOpen = ref(false);
    const audioToolPanel = ref<"speed" | "sleep" | null>(null);
    function openAudioToolPanel(panel: "speed" | "sleep"): void {
      audioToolPanel.value = panel;
      context.openReaderControls("tts");
    }
    function openAudioAdvanced(): void {
      audioToolPanel.value = null;
      context.openReaderControls("tts");
    }
    watch(() => context.readerControlsOpen.value, open => {
      if (!open) audioToolPanel.value = null;
    });
    const videoFullscreen = ref(false);
    const videoFullscreenError = ref("");
    type WebkitVideoElement = HTMLVideoElement & {
      webkitEnterFullscreen?: () => void;
      webkitDisplayingFullscreen?: boolean;
    };
    const videoFullscreenSupported = ref(typeof document !== "undefined" && document.fullscreenEnabled);
    function getVideoElement(): WebkitVideoElement | null {
      const media = context.getReaderMediaElement();
      // Media belongs to the iframe's DOM realm; instanceof against the parent realm fails.
      return media?.tagName.toLowerCase() === "video" ? media as WebkitVideoElement : null;
    }
    function updateVideoFullscreen(): void {
      const video = getVideoElement();
      videoFullscreen.value = Boolean(document.fullscreenElement === context.readerFrame.value?.parentElement
        || video?.webkitDisplayingFullscreen);
    }
    async function toggleVideoFullscreen(): Promise<void> {
      if (!videoFullscreenSupported.value || !context.readingVideoChapter.value) return;
      const container = context.readerFrame.value?.parentElement;
      const video = getVideoElement();
      if (!container || !video) return;
      videoFullscreenError.value = "";
      try {
        if (document.fullscreenElement === container) {
          await document.exitFullscreen();
        } else if (document.fullscreenEnabled && typeof container.requestFullscreen === "function") {
          await container.requestFullscreen();
        } else if (typeof video.webkitEnterFullscreen === "function") {
          // WebKit requires the direct click gesture, so no await before this call.
          video.webkitEnterFullscreen();
        } else {
          videoFullscreenError.value = "此设备暂不支持全屏，请使用播放器自带的按钮。";
          return;
        }
        updateVideoFullscreen();
      } catch (error) {
        videoFullscreenError.value = `当前设备无法切换全屏：${error instanceof Error ? error.message : String(error)}`;
      }
    }
    let listenedVideo: HTMLVideoElement | null = null;
    function onNativeVideoFullscreen(): void { updateVideoFullscreen(); }
    function detachNativeVideo(): void {
      listenedVideo?.removeEventListener("webkitbeginfullscreen", onNativeVideoFullscreen);
      listenedVideo?.removeEventListener("webkitendfullscreen", onNativeVideoFullscreen);
      listenedVideo = null;
    }
    function bindNativeVideo(): void {
      const video = getVideoElement();
      if (video === listenedVideo) return;
      detachNativeVideo();
      listenedVideo = video;
      // Prefer the bound iframe media element over a detached video probe; WKWebView implementations differ.
      videoFullscreenSupported.value = Boolean(document.fullscreenEnabled
        || typeof video?.webkitEnterFullscreen === "function");
      listenedVideo?.addEventListener("webkitbeginfullscreen", onNativeVideoFullscreen);
      listenedVideo?.addEventListener("webkitendfullscreen", onNativeVideoFullscreen);
    }
    watch(() => context.readingChapterHtml.value, () => {
      detachNativeVideo();
      videoFullscreen.value = false;
      videoFullscreenError.value = "";
    });
    onMounted(() => document.addEventListener("fullscreenchange", updateVideoFullscreen));
    onUnmounted(() => {
      document.removeEventListener("fullscreenchange", updateVideoFullscreen);
      detachNativeVideo();
    });
    const comicReadMode = ref<"vertical" | "paged">("vertical");
    const comicPanel = ref<"mode" | "fit" | "brightness" | null>(null);
    const comicPageIndex = ref(0);
    const comicPageCount = ref(0);
    let comicDocument: Document | null = null;
    let comicFrame: number | null = null;
    function updateComicCounter(): void {
      comicFrame = null;
      if (!comicDocument || !context.readingImagePage.value) return;
      const images = [...comicDocument.querySelectorAll<HTMLImageElement>("body img")]
        .filter(image => image.getBoundingClientRect().height > 0);
      comicPageCount.value = images.length;
      if (!images.length) { comicPageIndex.value = 0; return; }
      const reference = (comicDocument.documentElement.clientHeight || 0) * 0.45;
      const next = images.findIndex(image => image.getBoundingClientRect().bottom > reference);
      comicPageIndex.value = next < 0 ? images.length : next + 1;
    }
    function requestComicCounter(): void {
      if (comicFrame !== null) return;
      comicFrame = window.requestAnimationFrame(updateComicCounter);
    }
    function detachComicListeners(): void {
      if (comicFrame !== null) window.cancelAnimationFrame(comicFrame);
      comicFrame = null;
      comicDocument?.removeEventListener("scroll", requestComicCounter, true);
      comicDocument?.removeEventListener("load", requestComicCounter, true);
      comicDocument?.removeEventListener("touchstart", onComicTouchStart);
      comicDocument?.removeEventListener("touchend", onComicTouchEnd);
      comicDocument = null;
      comicPageIndex.value = 0;
      comicPageCount.value = 0;
    }
    watch(() => context.readingImagePage.value, visible => { if (!visible) detachComicListeners(); });
    onUnmounted(detachComicListeners);
    // This is a viewing preference, not a replacement for the resource/scroll owner.
    function applyComicMode(): void {
      const frame = context.readerFrame.value;
      if (!context.readingImagePage.value || !frame) return;
      const document = frame.contentDocument;
      if (!document?.head) return;
      let style = document.querySelector<HTMLStyleElement>("style[data-legado-comic-view]");
      if (!style) {
        style = document.createElement("style");
        style.dataset.legadoComicView = "true";
        document.head.appendChild(style);
      }
      style.textContent = comicReadMode.value === "paged"
        ? `html{scroll-snap-type:y mandatory!important;scroll-padding-top:0!important}
           body img{scroll-snap-align:start!important;scroll-snap-stop:always!important;max-height:100vh!important;width:auto!important;max-width:100%!important;object-fit:contain!important}`
        : "";
    }
    function onComicFrameLoad(event: Event): void {
      context.onReaderFrameLoad(event);
      bindNativeVideo();
      applyComicMode();
    }
    watch(comicReadMode, applyComicMode);
    function nextComicImage(direction: -1 | 1): void {
      const doc = context.readerFrame.value?.contentDocument;
      const scroller = doc?.scrollingElement;
      if (!doc || !scroller) return;
      const max = Math.max(0, scroller.scrollHeight - scroller.clientHeight);
      const offset = Math.max(0, scroller.scrollTop);
      if (comicReadMode.value === "vertical") {
        const step = Math.max(120, scroller.clientHeight * 0.86);
        const goal = Math.max(0, Math.min(max, offset + direction * step));
        if (Math.abs(goal - offset) > 1) scroller.scrollTo({ top: goal, behavior: "smooth" });
        else changeComicChapter(direction);
        return;
      }
      const imgs = [...doc.querySelectorAll<HTMLImageElement>("body img")].filter(img => img.getBoundingClientRect().height > 0);
      const targets = imgs.map(img => img.getBoundingClientRect().top + offset);
      const next = direction > 0
        ? targets.find(target => target > offset + 10)
        : targets.slice().reverse().find(target => target < offset - 10);
      if (next !== undefined) {
        scroller.scrollTo({ top: Math.min(max, Math.max(0, next)), behavior: "smooth" });
      } else if (direction < 0 && offset > 1) {
        scroller.scrollTo({ top: 0, behavior: "smooth" });
      } else if (direction > 0 && max - offset > 2) {
        scroller.scrollTo({ top: max, behavior: "smooth" });
      } else changeComicChapter(direction);
    }
    function changeComicChapter(direction: -1 | 1): void {
      const next = context.readingChapterIndex.value + direction;
      const count = context.readerChapterCount.value;
      if (next >= 0 && next < count) context.selectReaderDirectoryChapter(next);
    }
    function turnReaderEdge(direction: -1 | 1): void {
      if (context.readingImagePage.value) nextComicImage(direction);
      else context.turnPage(direction);
    }
    function openComicPanel(panel: "mode" | "fit" | "brightness"): void {
      comicPanel.value = panel;
      context.openReaderControls("appearance");
    }
    function onComicTouchStart(event: TouchEvent): void {
      if (!context.readingImagePage.value || comicReadMode.value !== "paged") return;
      const touch = event.touches.item(0);
      if (touch) comicTouchStart = { x: touch.clientX, y: touch.clientY };
    }
    function onComicTouchEnd(event: TouchEvent): void {
      const start = comicTouchStart;
      comicTouchStart = null;
      if (!start || comicReadMode.value !== "paged" || !context.readingImagePage.value) return;
      const touch = event.changedTouches.item(0);
      if (!touch) return;
      const dx = touch.clientX - start.x, dy = touch.clientY - start.y;
      if (Math.abs(dx) < 65 || Math.abs(dx) < Math.abs(dy) * 1.4) return;
      nextComicImage(dx < 0 ? 1 : -1);
    }
    let comicTouchStart: { x: number; y: number } | null = null;
    // The counter observes only the currently loaded display document; scroll/progress ownership stays in the reader binding.
    function onComicDocumentLoaded(event: Event): void {
      detachComicListeners();
      onComicFrameLoad(event);
      const doc = context.readerFrame.value?.contentDocument;
      if (context.readingImagePage.value && doc) {
        comicDocument = doc;
        doc.addEventListener("touchstart", onComicTouchStart, { passive: true });
        doc.addEventListener("touchend", onComicTouchEnd, { passive: true });
        doc.addEventListener("scroll", requestComicCounter, { capture: true, passive: true });
        doc.addEventListener("load", requestComicCounter, true);
        requestComicCounter();
      }
    }
    const videoChapters = computed(() => {
      const chapters = (context.readingBook.value?.chapters ?? []).slice().sort((a, b) => a.index - b.index);
      const current = chapters.findIndex(chapter => chapter.index === context.readingChapterIndex.value);
      const start = Math.max(0, Math.min(Math.max(0, chapters.length - 7), current - 2));
      return chapters.slice(start, start + 7);
    });
    const clampValue = (value: number, min: number, max: number): number =>
      Math.min(max, Math.max(min, value));
    return { ...context, ctx: proxyRefs(context), clampValue, comicBrightness, comicPageIndex, comicPageCount, pdfFindOpen, videoFullscreen, videoFullscreenError, videoFullscreenSupported, toggleVideoFullscreen, comicReadMode, comicPanel, audioToolPanel, openAudioToolPanel, openAudioAdvanced, nextComicImage, turnReaderEdge, openComicPanel, onComicDocumentLoaded, videoChapters };
  },
});
</script>

<template>
<section v-if="ctx.readerVisible" class="reader-shell" :data-reader-mode="ctx.readingPdfPage ? 'pdf' : ctx.readingVideoChapter ? 'video' : ctx.readingMediaChapter ? 'audio' : ctx.readingImagePage ? 'comic' : 'novel'" :class="[`reader-theme-${ctx.settings.reader.theme}`, { 'reader-menu-open': readerMenuOpen, 'reader-chrome-hidden': !readerMenuOpen, 'audio-console-active': ctx.readingMediaChapter && !ctx.readingVideoChapter && !readerBusy && !ctx.readerChapterError && !!ctx.readingChapterHtml }]" @click="onReaderSurfaceClick">
      <header class="reader-topbar">
        <div class="reader-toolbar-row">
          <component :is="ctx.BackButton" class="reader-back" label="返回书架" :disabled="readerSourceRefreshBusy" @click="closeReader()" />
          <div class="reader-book-heading"><strong>{{ ctx.readingBook?.title }}</strong><small>{{ currentChapterTitle }}</small></div>
          <div class="reader-header-actions">
            <span class="read-save-state" :class="progressSaveState">{{ progressSaveState === 'saving' ? '保存中' : progressSaveState === 'saved' ? '已保存' : progressSaveState === 'error' ? '保存失败' : '' }}</span>
            <button v-if="!ctx.readingMediaChapter && !ctx.readingPdfPage && !ctx.readingImagePage" type="button" class="reader-tool-button" @click.stop="ctx.openReaderControls('search')"><PrototypeIcon name="search"/></button>
            <button v-if="!ctx.readingMediaChapter && !ctx.readingPdfPage && !ctx.readingImagePage" type="button" class="reader-tool-button" @click.stop="ctx.toggleCurrentBookmark"><PrototypeIcon name="bookmark"/></button>
            <button v-if="!ctx.readingMediaChapter && !ctx.readingPdfPage && ctx.readingBook?.canChangeSource" type="button" class="reader-tool-button" :disabled="readerBusy || bookSourceSwitchBusy" @click.stop="openBookSourceSwitch(true)"><PrototypeIcon name="source-switch"/></button>
            <button type="button" class="reader-tool-button"
              @click.stop="ctx.readingMediaChapter ? openAudioAdvanced() : ctx.openReaderControls(ctx.readingPdfPage ? 'appearance' : 'settings')">
              <PrototypeIcon name="more"/>
            </button>
          </div>
        </div>
      </header>
      <div class="reader-main">
        <output v-if="ctx.readingImagePage && comicPageCount" class="reader-comic-page-counter"
>{{ comicPageIndex }} / {{ comicPageCount }}</output>
        <div v-if="ctx.readingVideoChapter && ctx.readerVideoQualityChoices.length > 1" class="reader-video-quality-control" @click.stop>
          <label for="reader-video-quality">清晰度</label>
          <select id="reader-video-quality" :value="ctx.readerVideoQualityIndex" :disabled="ctx.isReaderMediaControlBlocked()" @change="onReaderVideoQualityChange">
            <option v-for="(choice, index) in ctx.readerVideoQualityChoices" :key="index" :value="index" :disabled="!choice.src || !!choice.unavailableReason">{{ choice.label }}{{ choice.unavailableReason ? ('（' + choice.unavailableReason + '）') : '' }}</option>
          </select>
        </div>
        <div class="reader-content-frame" :style="ctx.readingImagePage ? { filter: `brightness(${comicBrightness}%)` } : undefined">
          <component :is="ctx.PdfReaderPage" v-if="ctx.readingPdfPage" :key="ctx.readingPdfPage.src" :src="ctx.readingPdfPage.src" :page-index="ctx.readingPdfPage.pageIndex" :default-zoom="ctx.activePdfZoom" :initial-password="ctx.readingPdfInitialPassword" @initial-password-used="consumeImportedPdfPassword" @loaded="ctx.readerPageCount = 1" :find-open="pdfFindOpen" @update:find-open="pdfFindOpen = $event" @page-selected="ctx.selectReaderDirectoryChapter($event)" @close-reader="closeReader" />
          <template v-else>
            <div v-if="readerBusy && !ctx.readerMediaRetrying" class="reader-loading"><span class="loader-ring"></span><p>{{ readerSourceRefreshBusy ? '正在从书源更新本章缓存…' : prefetchBusy ? '正在准备后续章节…' : '正在打开章节…' }}</p></div>
            <div v-else-if="ctx.readerChapterError" class="reader-loading reader-load-error">
              <strong>第 {{ (ctx.readerErrorChapterIndex ?? ctx.readingChapterIndex) + 1 }} 章暂时无法显示</strong>
              <p>{{ ctx.readerChapterError }}</p>
              <div class="reader-load-actions">
                <button type="button" class="button secondary small" :disabled="readerBusy" @click.stop="ctx.loadReaderChapter(ctx.readerErrorChapterIndex ?? ctx.readingChapterIndex)">重试缓存章节</button>
                <button v-if="ctx.readingBook?.sourceId && !ctx.readingMediaChapter && typeof ctx.refreshCurrentReaderChapter === 'function'" type="button" class="button secondary small" :disabled="readerBusy || readerSourceRefreshBusy || ctx.shelfBatchRecoveryRequired" @click.stop="ctx.refreshCurrentReaderChapter">从书源重新获取</button>
                <button v-if="ctx.readingBook?.canChangeSource && typeof ctx.openBookSourceSwitch === 'function'" type="button" class="button secondary small" :disabled="readerBusy || bookSourceSwitchBusy || ctx.shelfBatchRecoveryRequired" @click.stop="ctx.openBookSourceSwitch(true)">切换书源</button>
              </div>
            </div>
            <div v-else-if="!ctx.readingChapterHtml && !readerBusy" class="reader-loading reader-load-error">
              <strong>本章正文尚未显示</strong>
              <p>阅读内容没有生成，请重试；仍为空时可以从书源重新获取本章。</p>
              <div class="reader-load-actions">
                <button type="button" class="button secondary small" :disabled="readerBusy" @click.stop="ctx.loadReaderChapter(ctx.readerErrorChapterIndex ?? ctx.readingChapterIndex)">重试章节</button>
                <button v-if="ctx.readingBook?.sourceId && !ctx.readingMediaChapter && typeof ctx.refreshCurrentReaderChapter === 'function'" type="button" class="button secondary small" :disabled="readerBusy || readerSourceRefreshBusy || ctx.shelfBatchRecoveryRequired" @click.stop="ctx.refreshCurrentReaderChapter">从书源重新获取</button>
                <button v-if="ctx.readingBook?.canChangeSource && typeof ctx.openBookSourceSwitch === 'function'" type="button" class="button secondary small" :disabled="readerBusy || bookSourceSwitchBusy || ctx.shelfBatchRecoveryRequired" @click.stop="ctx.openBookSourceSwitch(true)">切换书源</button>
              </div>
            </div>
            <iframe v-else ref="readerFrame" class="chapter-frame" :allow="ctx.readingVideoChapter ? 'fullscreen' : undefined" :data-reader-format="ctx.readingVideoChapter ? 'video' : ctx.readingMediaChapter ? 'audio' : ctx.readingImagePage ? 'image' : 'html'" title="章节内容" sandbox="allow-same-origin" :srcdoc="ctx.readingChapterHtml" @load="onComicDocumentLoaded"></iframe>
          </template>
        </div>
        <section v-if="ctx.readingVideoChapter && !readerBusy && !ctx.readerChapterError && ctx.readingChapterHtml" class="reader-video-episodes" @click.stop>
          <header>
            <div><strong>{{ currentChapterTitle }}</strong><small>{{ ctx.readingBook?.title }} · {{ readerChapterCount }} 集</small></div>
            <button type="button" @click="ctx.openReaderDirectory">全部选集 <PrototypeIcon name="right"/></button>
          </header>
          <div class="reader-video-controls">
            <button type="button" class="reader-video-play-toggle"
              :disabled="ctx.isReaderMediaControlBlocked()"
              @click="ctx.toggleReaderMediaPlayback()">
              <PrototypeIcon :name="ctx.readerMediaPlaying ? 'pause' : 'play'"/>
            </button>
            <span class="reader-video-time">{{ ctx.formatReaderMediaTime(ctx.readerMediaTimeMs) }}</span>
            <input type="range" min="0" step="1000" :max="Math.max(1000,ctx.readerMediaDurationMs)"
              :value="ctx.readerMediaTimeMs"
              :disabled="ctx.isReaderMediaControlBlocked() || ctx.readerMediaDurationMs <= 0"
              @change="ctx.seekReaderMedia(Number(($event.target as HTMLInputElement).value) / 1000)" />
            <span class="reader-video-time">{{ ctx.readerMediaDurationMs > 0 ? ctx.formatReaderMediaTime(ctx.readerMediaDurationMs) : '—:—' }}</span>
            <div class="reader-video-extra-controls">
              <label class="reader-video-rate-select">
                <span>倍速</span>
                <select
                  :value="ctx.readerVideoPlaybackRate" :disabled="ctx.isReaderMediaControlBlocked()"
                  @change="ctx.setReaderVideoPlaybackRate(Number(($event.target as HTMLSelectElement).value))">
                  <option v-for="rate in ([0.5,0.75,1,1.25,1.5,1.75,2,2.25,2.5,2.75,3] as const)"
                    :key="rate" :value="rate">{{ rate }}×</option>
                </select>
              </label>
              <button type="button" class="reader-video-fullscreen-toggle"
                :title="videoFullscreenSupported ? '全屏显示视频画面' : '此 WebView 不支持此全屏方式'"
                :disabled="!videoFullscreenSupported || ctx.isReaderMediaControlBlocked()"
                @click="toggleVideoFullscreen"><PrototypeIcon name="fullscreen"/>{{ videoFullscreen ? '退出全屏' : '全屏' }}</button>
              <button type="button" class="reader-video-episode-button" @click="ctx.openReaderDirectory()"><PrototypeIcon name="list"/> 选集</button>
            </div>
          </div>
          <p v-if="videoFullscreenError" class="reader-video-fullscreen-error">{{ videoFullscreenError }}</p>
          <nav v-if="videoChapters.length">
            <button v-for="chapter in videoChapters" :key="chapter.id" type="button"
              :class="{ active: chapter.index === ctx.readingChapterIndex }"
              :disabled="readerBusy || ctx.shelfBatchRecoveryRequired"
              :title="chapter.title" @click="ctx.selectReaderDirectoryChapter(chapter.index)">
              {{ chapter.index + 1 }}
            </button>
          </nav>
        </section>
        <article v-if="ctx.readingMediaChapter && !ctx.readingVideoChapter && !readerBusy && !ctx.readerChapterError && ctx.readingChapterHtml" class="reader-audio-console" @click.stop>
          <img v-if="ctx.readingBook?.coverSrc" class="reader-audio-art" :src="ctx.readingBook.coverSrc"/>
          <div v-else class="reader-audio-art reader-audio-fallback"><PrototypeIcon name="audio"/></div>
          <div class="reader-audio-body">
            <small>{{ ctx.readingBook?.author || '音频内容' }}</small>
            <h2>{{ ctx.readingBook?.title }}</h2>
            <strong class="reader-audio-episode">{{ currentChapterTitle }}</strong>
            <div class="reader-audio-progress">
              <span>{{ ctx.formatReaderMediaTime(ctx.readerMediaTimeMs) }}</span>
              <input type="range" min="0" step="1000" :max="Math.max(1000,ctx.readerMediaDurationMs)" :value="ctx.readerMediaTimeMs"
                :disabled="ctx.isReaderMediaControlBlocked() || ctx.readerMediaDurationMs <= 0"
@change="ctx.seekReaderMedia(Number(($event.target as HTMLInputElement).value) / 1000)" />
              <span>{{ ctx.readerMediaDurationMs > 0 ? ctx.formatReaderMediaTime(ctx.readerMediaDurationMs) : '—:—' }}</span>
            </div>
            <div class="reader-audio-playback">
              <button type="button" :disabled="ctx.isReaderMediaControlBlocked()" @click="ctx.skipReaderMedia(-ctx.settings.reader.audioSkipSeconds)"><PrototypeIcon name="back15"/></button>
              <button type="button" class="reader-audio-main-play" :disabled="ctx.isReaderMediaControlBlocked()" @click="ctx.toggleReaderMediaPlayback()">
                <PrototypeIcon :name="ctx.readerMediaPlaying ? 'pause' : 'play'"/>
              </button>
              <button type="button" :disabled="ctx.isReaderMediaControlBlocked()" @click="ctx.skipReaderMedia(ctx.settings.reader.audioSkipSeconds)"><PrototypeIcon name="forward15"/></button>
            </div>
            <div class="reader-audio-utilities">
              <button type="button" @click="openAudioToolPanel('speed')"><strong>{{ ctx.settings.reader.audioPlaybackRate }}×</strong><small>倍速</small></button>
              <button type="button" @click="openAudioToolPanel('sleep')"><PrototypeIcon name="moon"/><small>{{ readerMediaSleepTimerMinutes ? readerMediaSleepTimerMinutes + ' 分钟' : '睡眠定时' }}</small></button>
              <button type="button" @click="ctx.openReaderDirectory"><PrototypeIcon name="list"/><small>节目</small></button>
            </div>
          </div>
        </article>
        <div v-if="ctx.readingMediaChapter && ctx.readerMediaError" class="reader-media-error">
          <span>{{ ctx.readerMediaError }}</span>
          <button type="button" class="button secondary small" :disabled="readerBusy || ctx.readerMediaRetrying || ctx.shelfBatchRecoveryRequired" @click.stop="retryReaderMedia">{{ ctx.readerMediaRetrying ? '正在重新加载…' : '重新加载' }}</button>
        </div>
        <div v-if="!ctx.readingMediaChapter" class="reader-edge reader-edge-left" @click.stop="turnReaderEdge(-1)"></div>
        <div v-if="!ctx.readingMediaChapter" class="reader-edge reader-edge-right" @click.stop="turnReaderEdge(1)"></div>
      </div>
      <footer v-if="ctx.readingImagePage" class="reader-footer reader-comic-dock">
        <nav class="reader-bottom-nav">
          <button type="button" class="reader-nav-action" @click="ctx.openReaderDirectory"><span><PrototypeIcon name="list"/></span><small>选话</small></button>
          <button type="button" class="reader-nav-action" @click="openComicPanel('mode')"><span><PrototypeIcon name="layout"/></span><small>阅读模式</small></button>
          <button type="button" class="reader-nav-action" @click="openComicPanel('fit')"><span><PrototypeIcon name="appearance"/></span><small>图片适配</small></button>
          <button type="button" class="reader-nav-action" @click="openComicPanel('brightness')"><span><PrototypeIcon name="sun"/></span><small>亮度</small></button>
        </nav>
      </footer>
      <footer v-if="ctx.readingPdfPage" class="reader-footer reader-pdf-dock">
        <nav class="reader-bottom-nav">
          <button type="button" class="reader-nav-action" @click="ctx.openReaderDirectory"><span><PrototypeIcon name="list"/></span><small>页面</small></button>
          <button type="button" class="reader-nav-action" @click="pdfFindOpen = !pdfFindOpen"><span><PrototypeIcon name="search"/></span><small>搜索</small></button>
          <button type="button" class="reader-nav-action" @click="ctx.openReaderControls('appearance')"><span><PrototypeIcon name="appearance"/></span><small>显示</small></button>
          <button type="button" class="reader-nav-action" @click="ctx.toggleCurrentBookmark"><span><PrototypeIcon name="bookmark"/></span><small>{{ currentBookmark ? '已加书签' : '书签' }}</small></button>
        </nav>
      </footer>
      <footer v-if="!ctx.readingMediaChapter && !ctx.readingImagePage && !ctx.readingPdfPage" class="reader-footer">
        <nav class="reader-bottom-nav">
          <button type="button" class="reader-nav-action" @click="ctx.openReaderDirectory"><span><PrototypeIcon name="list"/></span><small>{{ ctx.readingPdfPage ? '页面' : ctx.readingVideoChapter ? '剧集' : ctx.readingMediaChapter ? '节目' : ctx.readingImagePage ? '选话' : '目录' }}</small></button>
          <button v-if="!ctx.readingPdfPage && !ctx.readingImagePage && !ctx.readingVideoChapter" type="button" class="reader-nav-action" :class="{ active: ctx.readerControlsOpen && readerControlsMode === 'tts' }" @click="openTtsControls"><span><PrototypeIcon name="tts"/></span><small>{{ ctx.readingMediaChapter ? '倍速与定时' : '朗读' }}</small></button>
          <button v-if="!ctx.readingMediaChapter" type="button" class="reader-nav-action" :class="{ active: ctx.readerControlsOpen && readerControlsMode === 'appearance' }" @click="ctx.openReaderControls('appearance')"><span class="reader-appearance-glyph">Aa</span><small>{{ ctx.readingPdfPage ? '缩放' : ctx.readingImagePage ? '图片适配' : '显示' }}</small></button>
          <!-- The header's More action owns advanced settings, as in the prototype. -->
        </nav>
      </footer>
      <div v-if="readerDirectoryOpen" class="reader-directory-backdrop" @click.self="readerDirectoryOpen = false">
        <aside class="reader-directory-panel" @click.stop>
          <header class="reader-directory-header">
            <div><small>{{ ctx.readingPdfPage ? 'PDF 页面' : ctx.readingVideoChapter ? '视频选集' : ctx.readingMediaChapter ? '音频节目' : ctx.readingImagePage ? '漫画选话' : '章节目录' }}</small><strong>{{ ctx.readingBook?.title }}</strong><span>{{ ctx.readingBook?.chapters?.length ?? 0 }} / {{ readerChapterCount }} {{ ctx.readingPdfPage ? '页' : ctx.readingVideoChapter ? '集' : ctx.readingMediaChapter ? '期' : ctx.readingImagePage ? '话' : '章' }}</span></div>
            <button type="button" @click="readerDirectoryOpen = false"><PrototypeIcon name="close"/></button>
          </header>
          <label class="reader-directory-search"><PrototypeIcon name="search"/><input v-model="readerDirectoryQuery" type="search" :placeholder="ctx.readingPdfPage ? '搜索页码或页名' : ctx.readingVideoChapter ? '搜索剧集' : ctx.readingMediaChapter ? '搜索节目' : ctx.readingImagePage ? '搜索话数' : '搜索章节名称'" /></label>
          <div class="reader-directory-list">
            <button v-for="chapter in filteredReaderChapters" :key="chapter.id" type="button" class="reader-directory-chapter" :class="{ current: chapter.index === ctx.readingChapterIndex }" :disabled="readerBusy" @click="ctx.selectReaderDirectoryChapter(chapter.index)">
              <span class="reader-directory-number">{{ String(chapter.index + 1).padStart(2, '0') }}</span>
              <strong>{{ chapter.title }}</strong>
              <span v-if="chapter.index === ctx.readingChapterIndex" class="reader-directory-current">{{ ctx.readingVideoChapter ? '正在播放' : ctx.readingMediaChapter ? '正在收听' : '当前位置' }}</span>
            </button>
            <p v-if="!filteredReaderChapters.length" class="reader-directory-empty">没有匹配的章节。</p>
          </div>
        </aside>
      </div>
      <aside v-if="ctx.readerControlsOpen" class="reader-settings-popover" :class="{ 'reader-search-popover': readerControlsMode === 'search' }">
        <div class="popover-title"><span>{{ readerControlsMode === 'settings' ? '更多阅读选项' : readerControlsMode === 'tts' ? (ctx.readingMediaChapter ? audioToolPanel === 'speed' ? '播放速度' : audioToolPanel === 'sleep' ? '睡眠定时' : '播放设置' : '朗读') : readerControlsMode === 'appearance' ? ctx.readingImagePage ? comicPanel === 'mode' ? '阅读模式' : comicPanel === 'fit' ? '图片适配' : '亮度' : ctx.readingPdfPage ? 'PDF 显示' : '显示' : '搜索' }}</span><button @click="ctx.readerControlsOpen = false"><PrototypeIcon name="close"/></button></div>
        <!-- Chapter seek is available in advanced tools; the reading dock keeps only prototype primary actions. -->
        <div v-if="readerControlsMode === 'settings' && !ctx.readingMediaChapter && !ctx.readingImagePage && !ctx.readingPdfPage" class="reader-settings-progress">
          <div class="reader-chapter-progress">
            <div class="reader-chapter-position">{{ currentChapterTitle }} <span>（{{ ctx.readingChapterIndex + 1 }} / {{ readerChapterCount }}）</span></div>
          <div v-if="readerChapterCount > 1" class="reader-chapter-seek-row">
            <input type="range" min="0" :max="readerChapterCount - 1" step="1" :value="ctx.readingChapterIndex" :disabled="readerBusy || ctx.shelfBatchRecoveryRequired" @change="ctx.jumpReaderChapterSlider" />
          </div>
        </div>

        </div>
        <div v-if="readerControlsMode === 'settings' && !ctx.readingImagePage && !ctx.readingPdfPage" class="setting-control compact-setting">
          <div><label for="reader-chapter-jump">跳转章节</label><small>共 {{ ctx.readingBook?.chapters?.length ?? 0 }} 章</small></div>
          <input id="reader-chapter-jump" type="number" inputmode="numeric" min="1" :max="ctx.readingBook?.chapters?.length ?? 0" :value="ctx.readingChapterIndex + 1" :disabled="readerBusy || ctx.shelfBatchRecoveryRequired" @change="jumpReaderChapter" />
        </div>
        <button v-if="readerControlsMode === 'settings'" type="button" class="button secondary small reader-add-bookmark" @click="toggleCurrentBookmark">{{ currentBookmark ? '移除当前书签' : '添加当前页书签' }}</button>
        <div v-if="readerControlsMode === 'appearance' && !ctx.readingMediaChapter && !ctx.readingPdfPage && !ctx.readingImagePage" class="setting-control compact-setting">
          <div><label for="reader-scroll-mode">阅读方式</label><small>本章按屏幕滚动或分页</small></div>
          <select id="reader-scroll-mode" :value="ctx.settings.reader.verticalScroll ? 'vertical' : 'paged'" :disabled="readerBusy" @change="updateReaderSetting('verticalScroll', ($event.target as HTMLSelectElement).value === 'vertical')">
            <option value="paged">左右翻页</option><option value="vertical">上下滚动</option>
          </select>
        </div>
        <div v-if="readerControlsMode === 'tts' && ctx.readingMediaChapter && !ctx.readingVideoChapter && !audioToolPanel" class="setting-control reader-audio-rate-control">
          <div><label for="reader-audio-rate">播放速度</label><small>{{ ctx.settings.reader.audioPlaybackRate }}×</small></div>
          <input id="reader-audio-rate" type="range" min="0.5" max="3" step="0.25" :value="ctx.settings.reader.audioPlaybackRate" @input="updateReaderSetting('audioPlaybackRate', Number(($event.target as HTMLInputElement).value))" />
        </div>
        <div v-if="readerControlsMode === 'tts' && ctx.readingMediaChapter && !ctx.readingVideoChapter && audioToolPanel === 'speed'" class="reader-audio-choice-grid">
          <button v-for="rate in ([0.75, 1, 1.25, 1.5, 1.75, 2] as const)" :key="rate" type="button"
            :class="{ selected: Math.abs(ctx.settings.reader.audioPlaybackRate - rate) < 0.001 }"
            :disabled="ctx.isReaderMediaControlBlocked()"
            @click="updateReaderSetting('audioPlaybackRate', rate)">{{ rate }}×</button>
        </div>
        <div v-if="readerControlsMode === 'tts' && ctx.readingMediaChapter && audioToolPanel !== 'speed'" class="setting-control compact-setting">
          <div><label for="reader-media-sleep-timer">睡眠定时器</label><small>时间到后暂停并保存位置</small></div>
          <select v-if="audioToolPanel !== 'sleep'" id="reader-media-sleep-timer" :value="readerMediaSleepTimerMinutes" :disabled="ctx.isReaderMediaControlBlocked()" @change="onReaderMediaSleepTimerChange">
            <option :value="0">关闭</option><option :value="15">15 分钟</option><option :value="30">30 分钟</option><option :value="60">60 分钟</option>
          </select>
          <div v-else class="reader-audio-sleep-choices">
            <button v-for="minutes in ([0, 15, 30, 60] as const)" :key="minutes" type="button"
              :class="{ selected: readerMediaSleepTimerMinutes === minutes }"
              :disabled="ctx.isReaderMediaControlBlocked()"
              @click="ctx.setReaderMediaSleepTimer(minutes)">
              <span><strong>{{ minutes === 0 ? '关闭' : minutes + ' 分钟' }}</strong><small v-if="minutes === 0">不自动暂停</small></span>
              <span class="reader-choice-radio"></span>
            </button>
          </div>
        </div>
        <div v-if="readerControlsMode === 'settings' && !ctx.readingMediaChapter && !ctx.readingPdfPage" class="reader-source-tools">
          <button v-if="!ctx.readingImagePage && ctx.readingBook?.canChangeSource" type="button" class="button secondary small" :disabled="readerBusy || readerSourceRefreshBusy || bookSourceSwitchBusy" @click.stop="ctx.refreshCurrentReaderChapter"><PrototypeIcon name="refresh"/>从书源重新获取本章</button>
          <button v-if="ctx.readingBook?.sourceId" type="button" class="button secondary small" :disabled="readerBusy || readerBookDownloadBusy || ctx.shelfBatchRecoveryRequired" @click.stop="ctx.downloadReaderBook"><PrototypeIcon name="download"/>缓存全部章节</button>
        </div>
        <button v-if="readerControlsMode === 'settings' && !ctx.readingMediaChapter && !ctx.readingPdfPage && !ctx.readingImagePage" type="button" class="button secondary small reader-dictionary-open" @click="openReaderDictionary">词典查词</button>
        <div v-if="readerControlsMode === 'tts' && !ctx.readingMediaChapter && !ctx.readingPdfPage && !ctx.readingImagePage" class="reader-tts-panel">
          <div class="reader-tts-description"><label for="reader-tts-engine">朗读引擎</label><small v-if="ttsControlStatus">{{ ttsControlStatus }}</small><small v-if="ttsControlPlayback !== 'idle' && ttsControlProgress">片段 {{ ttsControlProgress.current }} / {{ ttsControlProgress.total }}</small></div>
          <select id="reader-tts-engine" :value="ttsEngineSelection" :disabled="ttsControlPlayback !== 'idle' || readerBusy" @change="selectTtsEngine(($event.target as HTMLSelectElement).value)">
            <optgroup label="本地语音">
              <option value="system:">系统默认语音</option>
              <option v-if="ctx.settings.reader.ttsVoiceURI && !ttsVoices.some((voice) => voice.voiceURI === ctx.settings.reader.ttsVoiceURI)" :value="`system:${ctx.settings.reader.ttsVoiceURI}`">已保存语音不可用（系统默认）</option>
              <option v-for="voice in ttsVoices" :key="voice.voiceURI" :value="`system:${voice.voiceURI}`">{{ voice.name }}{{ voice.lang ? `（${voice.lang}）` : '' }}</option>
            </optgroup>
            <optgroup label="网络朗读">
              <option v-if="ctx.settings.reader.ttsEngine === 'http' && ctx.settings.reader.httpTtsConfigId && !ctx.httpTtsConfigs.some((item) => item.id === ctx.settings.reader.httpTtsConfigId)" :value="`http:${ctx.settings.reader.httpTtsConfigId}`" disabled>当前网络朗读配置不可用</option>
              <option v-if="!ctx.httpTtsConfigs.length" value="http:" disabled>尚未添加网络朗读配置</option>
              <option v-for="item in ctx.httpTtsConfigs" :key="item.id" :value="`http:${item.id}`" :disabled="!item.enabled">{{ item.name }}{{ item.enabled ? '' : '（已停用）' }}</option>
            </optgroup>
          </select>
          <button v-if="!ctx.httpTtsConfigs.some((item) => item.enabled)" type="button" class="text-button" @click="ctx.openHttpTtsSettings">添加网络朗读源</button>
          <div class="reader-tts-row"><label for="reader-tts-start-position">朗读起点</label><select id="reader-tts-start-position" :value="ctx.settings.reader.ttsStartPosition" @change="updateReaderSetting('ttsStartPosition', ($event.target as HTMLSelectElement).value === 'chapter' ? 'chapter' : 'current')"><option value="current">当前位置</option><option value="chapter">本章开头</option></select></div>
          <div class="reader-tts-row reader-tts-rate"><label for="reader-tts-rate">语速</label><input id="reader-tts-rate" type="range" min="0.5" max="5" step="0.1" :value="ctx.settings.reader.ttsRate" @input="setTtsRate(Number(($event.target as HTMLInputElement).value))" /><small>{{ ctx.settings.reader.ttsRate.toFixed(1) }}×</small></div>
          <label class="reader-tts-continuous"><input :checked="ctx.settings.reader.ttsContinueAcrossChapters" type="checkbox" @change="updateReaderSetting('ttsContinueAcrossChapters', ($event.target as HTMLInputElement).checked)" />连续朗读并自动进入下一章</label>
          <div v-if="ctx.settings.reader.ttsEngine === 'system'" class="reader-tts-selection"><small>{{ ttsSelectedText ? `已选中 ${ttsSelectedText.length} 个字` : '先在正文中选中文字' }}</small><button v-if="ttsControlPlayback === 'idle'" type="button" class="button secondary small" :disabled="!ttsCanStart || !ttsSelectedText" @click="startSelectedTextTts">朗读选中文字</button></div>
          <div class="reader-tts-actions">
            <button type="button" class="button secondary small" :disabled="ttsControlPlayback === 'loading' || (ttsControlPlayback === 'idle' && !ttsControlCanStart)" @click="ttsControlPlayback === 'speaking' ? pauseSelectedTts() : ttsControlPlayback === 'paused' ? resumeSelectedTts() : startSelectedTts()">{{ ttsControlPlayback === 'loading' ? '准备中…' : ttsControlPlayback === 'speaking' ? '暂停' : ttsControlPlayback === 'paused' ? '继续' : '开始朗读' }}</button>
            <button v-if="ttsControlPlayback !== 'idle'" type="button" class="button secondary small" @click="stopSelectedTts()">停止</button>
          </div>
          <p v-if="ctx.settings.reader.ttsEngine === 'system' && ttsError">{{ ttsError }}</p>
          <p v-else-if="ctx.settings.reader.ttsEngine === 'http' && ttsControlError">{{ ttsControlError }}</p>
          <button v-if="ctx.settings.reader.ttsEngine === 'system' && ttsControlPlayback === 'idle' && ttsAvailability !== 'ready'" type="button" class="button secondary small" @click="retryTtsAvailability">重新检测系统语音</button>
        </div>
        <div v-if="readerControlsMode === 'settings' && currentBookmark" class="reader-current-bookmark">
          <span>{{ ctx.readingMediaChapter ? (ctx.readingVideoChapter ? '当前视频位置已添加书签' : '当前音频位置已添加书签') : '当前页已添加书签' }}</span>
          <button type="button" class="button secondary small" @click="editCurrentBookmarkNote">编辑备注</button>
        </div>
        <div v-if="readerControlsMode === 'search' && !ctx.readingMediaChapter && !ctx.readingPdfPage && !ctx.readingImagePage" class="setting-control reader-find-control">
          <input id="reader-find-input" v-model="readerFindQuery" type="search" autocomplete="off" placeholder="输入要查找的文字" @keydown="handleReaderFindKeydown" />
          <div class="reader-find-actions-row">
            <small>{{ readerFindQuery.trim() ? readerFindMatches.length ? `${readerFindIndex + 1} / ${readerFindMatches.length}` : '无匹配' : '' }}</small>
            <div class="reader-find-buttons">
            <button type="button" class="button secondary small" :disabled="!readerFindMatches.length" @click="moveReaderFind(-1)">上一处</button>
            <button type="button" class="button secondary small" :disabled="!readerFindMatches.length" @click="moveReaderFind(1)">下一处</button>
            </div>
          </div>
        </div>
        <div v-if="readerControlsMode === 'appearance' && ctx.readingPdfPage" class="pdf-scale-control">
          <span>PDF 缩放</span>
          <div>
            <button :class="{ selected: ctx.activePdfZoom === 'page-fit' }" @click="setPdfZoom('page-fit')">整页</button>
            <button :class="{ selected: ctx.activePdfZoom === 'page-width' }" @click="setPdfZoom('page-width')">适合宽度</button>
            <button :class="{ selected: ctx.activePdfZoom === 'actual-size' }" @click="setPdfZoom('actual-size')">原始大小</button>
          </div>
        </div>
        <div v-if="readerControlsMode === 'appearance' && ctx.readingImagePage && comicPanel === 'mode'" class="comic-mode-control">
          <button type="button" :class="{ selected: comicReadMode === 'vertical' }" @click="comicReadMode = 'vertical'"><strong>上下连续</strong><small>条漫和连续滚动阅读</small></button>
          <button type="button" :class="{ selected: comicReadMode === 'paged' }" @click="comicReadMode = 'paged'"><strong>左右翻页</strong><small>单页定位，点击两侧或左右滑动切换</small></button>
        </div>
        <div v-if="readerControlsMode === 'appearance' && ctx.readingImagePage && comicPanel === 'brightness'" class="comic-brightness-control">
          <label for="comic-brightness">亮度</label>
          <input id="comic-brightness" v-model.number="comicBrightness" type="range" min="50" max="150" step="5"/>
          <strong>{{ comicBrightness }}%</strong>
        </div>
        <div v-if="readerControlsMode === 'appearance' && ctx.readingImagePage && comicPanel === 'fit'" class="comic-scale-control">
          <span>图片缩放</span>
          <div>
            <button :class="{ selected: ctx.comicScaleMode === 'fit-width' }" @click="setComicScaleMode('fit-width')">适合宽度</button>
            <button :class="{ selected: ctx.comicScaleMode === 'actual-size' }" @click="setComicScaleMode('actual-size')">原始大小</button>
          </div>
        </div>
        <template v-if="readerControlsMode === 'appearance' && !ctx.readingMediaChapter && !ctx.readingPdfPage && !ctx.readingImagePage">
          <div class="setting-control compact-setting">
            <div><label>字号</label><small>{{ ctx.settings.reader.fontSizePx }} px</small></div>
            <div class="range-control">
              <button @click="updateReaderSetting('fontSizePx', clampValue(ctx.settings.reader.fontSizePx - 1, 12, 36))">−</button>
              <input type="range" min="12" max="36" :value="ctx.settings.reader.fontSizePx" @input="updateReaderSetting('fontSizePx', Number(($event.target as HTMLInputElement).value))" />
              <button @click="updateReaderSetting('fontSizePx', clampValue(ctx.settings.reader.fontSizePx + 1, 12, 36))">＋</button>
            </div>
          </div>
          <div class="setting-control compact-setting">
            <div><label>行间距</label><small>{{ ctx.settings.reader.lineHeight.toFixed(1) }}</small></div>
            <input type="range" min="1.2" max="2.8" step="0.1" :value="ctx.settings.reader.lineHeight" @input="updateReaderSetting('lineHeight', Number(($event.target as HTMLInputElement).value))" />
          </div>
        </template>
        <div v-if="readerControlsMode === 'appearance' && !ctx.readingImagePage && !ctx.readingPdfPage && !ctx.readingMediaChapter" class="setting-control compact-setting">
          <div><label>阅读主题</label></div>
          <div class="theme-options"><button v-for="theme in ([['paper','纸'],['sepia','暖'],['dark','夜'],['system','白']] as const)" :key="theme[0]" class="theme-option compact-theme" :class="[`theme-${theme[0]}`, { selected: ctx.settings.reader.theme === theme[0] }]" @click="setReaderTheme(theme[0])">{{ theme[1] }}</button></div>
        </div>
        <button v-if="readerControlsMode === 'settings'" class="text-button popover-save" :disabled="saveSettingsBusy" @click="persistSettings">{{ saveSettingsBusy ? '保存中…' : '保存阅读设置' }}</button>
      </aside>
      <!-- 播放器放在弹窗外，关闭朗读面板时不要卸载正在播放的音频。 -->
      <audio v-if="ctx.settings.reader.ttsEngine === 'http' && httpTtsAudioResource" :ref="setHttpTtsAudioElement" preload="none" :src="httpTtsAudioResource.src" class="reader-audio-element" @play="onHttpTtsAudioPlay" @pause="onHttpTtsAudioPause" @ended="onHttpTtsAudioEnded" @error="onHttpTtsAudioError"></audio>
    </section>

    <section v-if="ctx.readerVisible && ctx.dictionaryOpen" class="modal-backdrop reader-dictionary-backdrop" @click.self="ctx.closeReaderDictionary">
      <article class="dictionary-panel">
        <header>
          <div><span>{{ ctx.dictionaryProvider || '维基词典' }}</span><h2>{{ ctx.dictionaryTitle || '词典查词' }}</h2></div>
          <button class="detail-close" @click="ctx.closeReaderDictionary">×</button>
        </header>
        <form class="reader-dictionary-form" @submit.prevent="ctx.lookupReaderDictionary">
          <div class="reader-dictionary-row">
            <input v-model="ctx.dictionaryWord" class="reader-dictionary-word" maxlength="128" autocomplete="off" placeholder="输入词语，最多 128 个字符"/>
            <button type="button" class="button secondary small" @click="ctx.useSelectedReaderText">取选中文字</button>
          </div>
          <div class="reader-dictionary-row">
            <label for="reader-dictionary-language" class="reader-dictionary-language-label">词典</label>
            <select id="reader-dictionary-language" v-model="ctx.dictionaryLanguage" class="reader-dictionary-language">
              <option value="en">英文维基词典</option>
              <option value="zh">中文维基词典</option>
            </select>
            <button type="submit" class="button primary small" :disabled="ctx.dictionaryBusy">{{ ctx.dictionaryBusy ? '查询中…' : '查询' }}</button>
            <button v-if="ctx.dictionaryError" type="button" class="button secondary small" :disabled="ctx.dictionaryBusy" @click="ctx.lookupReaderDictionary">重试</button>
          </div>
          <p v-if="ctx.dictionaryError" class="reader-dictionary-error">{{ ctx.dictionaryError }}</p>
        </form>
        <div v-if="ctx.dictionarySourceUrl" class="reader-dictionary-source">
          <span>来源：</span><a :href="ctx.dictionarySourceUrl" target="_blank" rel="noopener noreferrer">{{ ctx.dictionarySourceUrl }}</a>
        </div>
        <div v-if="ctx.dictionaryBusy" class="reader-loading reader-dictionary-status"><span class="loader-ring"></span><p>正在查询维基词典…</p></div>
        <iframe v-else-if="ctx.dictionaryHtml" title="维基词典词条内容" sandbox="allow-same-origin" :srcdoc="ctx.dictionaryHtml"></iframe>
        <div v-else class="reader-loading reader-dictionary-status"><p>可输入词语，或先在章节内容中选中文字。</p></div>
      </article>
    </section>

    <section v-if="ctx.bookmarkEditorOpen" class="modal-backdrop reader-bookmark-backdrop"
      @click.self="!ctx.bookmarkBusy && ctx.closeBookmarkEditor()"
      @keydown.esc="!ctx.bookmarkBusy && ctx.closeBookmarkEditor()">
      <article class="app-modal bookmark-modal reader-bookmark-sheet">
        <header class="reader-bookmark-sheet-heading">
          <strong id="reader-bookmark-sheet-title">书签</strong>
          <button type="button" :disabled="ctx.bookmarkBusy"
            @click="ctx.closeBookmarkEditor"><PrototypeIcon name="close"/></button>
        </header>
        <p class="reader-bookmark-sheet-position">
          {{ ctx.readingBook?.title }} · {{ ctx.currentChapterTitle }} · {{ ctx.readerPositionDescription }}
        </p>
        <label class="reader-bookmark-sheet-label" for="reader-bookmark-note">备注（可选）</label>
        <textarea id="reader-bookmark-note" v-model="ctx.bookmarkNote" maxlength="300" autofocus
          placeholder="写下想记住的内容（可选）" :disabled="ctx.bookmarkBusy"></textarea>
        <div class="modal-actions">
          <button type="button" class="button secondary" :disabled="ctx.bookmarkBusy" @click="ctx.closeBookmarkEditor">取消</button>
          <button type="button" class="button primary" :disabled="ctx.bookmarkBusy"
            @click="ctx.saveBookmark">{{ ctx.bookmarkBusy ? '保存中…' : ctx.bookmarkEditingId ? '保存备注' : '保存书签' }}</button>
        </div>
      </article>
    </section>

    <BookSourceSwitchSheet
      v-if="ctx.readerVisible"
      :book-source-switch="bookSourceSwitch"
      :book-source-candidates="bookSourceCandidates"
      :source-name="sourceName"
      :shelf-batch-recovery-required="shelfBatchRecoveryRequired"
    />
</template>

<style scoped>

.reader-shell {
  position: fixed;
  z-index: 55;
  inset: 0;
  overflow: hidden;
  color: #3f3b34;
  background: #f7f3e9;
  --reader-chrome-bg: #fbfaf6;
  --reader-chrome-border: #e9e4da;
  --reader-chrome-text: #6d6a60;
  --reader-accent: var(--app-accent);
}
.reader-theme-sepia {
  color: #4e4131; background: #eee1c8;
  --reader-chrome-bg: #f5ead7; --reader-chrome-border: #e3d3b9;
  --reader-chrome-text: #766348; --reader-accent: var(--app-accent);
}
.reader-theme-dark {
  color: #d1d0cb; background: #17191c;
  --reader-chrome-bg: #1d2022; --reader-chrome-border: #303437;
  --reader-chrome-text: #adb1ad; --reader-accent: var(--app-accent);
}
.reader-theme-system {
  color: #252525; background: #fff;
  --reader-chrome-bg: #fff; --reader-chrome-border: #e6e8e6;
  --reader-chrome-text: #777; --reader-accent: var(--app-accent);
}

.reader-topbar, .reader-footer {
  position: absolute; z-index: 5;
  right: 0; left: 0;
  border-color: var(--reader-chrome-border);
  background: var(--reader-chrome-bg); color: var(--reader-chrome-text);
}
.reader-topbar { top: 0; height: calc(56px + env(safe-area-inset-top)); padding: env(safe-area-inset-top) 12px 0; border-bottom: 1px solid var(--reader-chrome-border); }
.reader-toolbar-row { display: grid; height: 56px; grid-template-columns: 44px minmax(0, 1fr) auto; align-items: center; gap: 8px; }
.reader-book-heading { display: flex; min-width: 0; flex-direction: column; }
.reader-book-heading strong, .reader-book-heading small { overflow: hidden; max-width: 100%; text-overflow: ellipsis; white-space: nowrap; }
.reader-header-actions { display: flex; align-items: center; }
.read-save-state { min-width: 42px; text-align: right; }
.reader-tool-button {
  display: grid; width: 36px;
  min-width: 36px; height: 36px;
  place-items: center; border-radius: 10px;
  color: var(--reader-chrome-text);
  cursor: pointer;
}
.reader-tool-button:hover:not(:disabled) {
  color: var(--app-accent-ink);
  background: var(--app-accent-soft);
}
.reader-tool-button:disabled {
  opacity: .45;
  cursor: default;
}

.reader-main {
  position: absolute; inset: 0;
  display: flex; min-height: 0;
  flex-direction: column; align-items: center;
  padding: 0 20px;
}
.reader-content-frame, .chapter-frame { width: 100%; height: 100%; }
.reader-content-frame { position: relative; flex: 1; min-height: 0; overflow: hidden; }
.chapter-frame { display: block; border: 0; }
.reader-loading { position: absolute; inset: 0; display: grid; place-content: center; gap: 10px; text-align: center; }
.reader-loading p { margin: 0; }
.reader-load-actions { display: flex; flex-wrap: wrap; justify-content: center; gap: 8px; }
.reader-video-quality-control { position: absolute; z-index: 4; top: 68px; right: 12px; display: flex; align-items: center; gap: 8px; }
.reader-media-error { position: absolute; z-index: 4; right: 10px; bottom: 68px; left: 10px; }
.reader-chrome-hidden .reader-topbar, .reader-chrome-hidden .reader-footer, .reader-chrome-hidden .reader-video-quality-control {
  display: none;
}

.reader-footer {
  bottom: 0; display: flex;
  min-height: calc(60px + env(safe-area-inset-bottom)); align-items: center;
  justify-content: center; padding: 0 10px env(safe-area-inset-bottom);
  border-top: 1px solid var(--reader-chrome-border);
}
.reader-bottom-nav { display: flex; width: 100%; min-height: 60px; align-items: center; justify-content: center; gap: 20px; }
.reader-nav-action { display: grid; min-width: 54px; min-height: 54px; place-items: center; gap: 2px; padding: 4px 8px; border: 0; border-radius: 12px; color: var(--reader-chrome-text); background: transparent; cursor: pointer; }
.reader-nav-action.active,
.reader-nav-action:hover:not(:disabled) {
  color: var(--app-accent-ink);
  background: var(--app-accent-soft);
}
.reader-nav-action > span { display: grid; width: 24px; height: 24px; place-items: center; }
.reader-comic-dock .reader-bottom-nav, .reader-pdf-dock .reader-bottom-nav { justify-content: space-around; }
.reader-comic-dock .reader-nav-action, .reader-pdf-dock .reader-nav-action { min-width: 0; flex: 1; }
.reader-comic-page-counter { position: absolute; z-index: 3; right: 20px; bottom: calc(70px + env(safe-area-inset-bottom)); }

.reader-settings-popover {
  position: absolute; z-index: 32;
  top: 68px; right: 12px;
  width: min(340px, calc(100vw - 24px)); max-height: min(68dvh, 680px);
  overflow: auto; box-sizing: border-box;
  padding: 14px; border: 1px solid var(--reader-chrome-border);
  border-radius: 16px; color: var(--reader-chrome-text);
  background: var(--reader-chrome-bg); box-shadow: var(--app-shadow);
  overscroll-behavior: contain;
}
.reader-search-popover { top: auto; bottom: calc(70px + env(safe-area-inset-bottom)); }
.popover-title, .reader-directory-header { display: flex; align-items: center; justify-content: space-between; gap: 10px; }
.popover-title { min-height: 34px; margin-bottom: 8px; color: var(--app-text); font-weight: 700; }
.popover-title > button,
.reader-directory-header > button {
  display: grid;
  width: 34px;
  height: 34px;
  flex: none;
  place-items: center;
  border-radius: 9px;
  color: var(--reader-chrome-text);
  background: var(--app-panel-soft);
  cursor: pointer;
}
.popover-title > button:hover,
.reader-directory-header > button:hover {
  color: var(--app-accent-ink);
  background: var(--app-accent-soft);
}
.reader-chapter-progress, .reader-audio-progress { display: grid; min-width: 0; gap: 8px; }
.reader-chapter-position { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.reader-chapter-seek-row input, .reader-audio-progress input { width: 100%; min-width: 0; margin: 0; accent-color: var(--reader-accent); }
.setting-control { display: grid; gap: 8px; padding: 8px 0; }
.compact-setting { grid-template-columns: minmax(72px, auto) minmax(0, 1fr); align-items: center; gap: 10px; }
.compact-setting > div:first-child { display: flex; flex-direction: column; }
.range-control { display: flex; min-width: 0; align-items: center; gap: 7px; }
.range-control > button {
  width: 34px;
  height: 34px;
  flex: none;
  border: 1px solid var(--reader-chrome-border);
  border-radius: 9px;
  color: var(--reader-chrome-text);
  background: var(--reader-chrome-bg);
  cursor: pointer;
}
.range-control > button:hover {
  border-color: var(--app-accent);
  color: var(--app-accent-ink);
  background: var(--app-accent-soft);
}
.range-control input { min-width: 0; flex: 1; }
.theme-options { display: flex; justify-content: flex-end; gap: 4px; }

.reader-directory-backdrop {
  position: absolute; z-index: 34;
  inset: 0; display: flex;
  align-items: flex-end; justify-content: flex-end;
  padding: 12px 12px calc(70px + env(safe-area-inset-bottom)); background: rgb(0 0 0 / 24%);
}
.reader-directory-panel {
  display: flex; width: min(340px, 100%);
  height: min(68dvh, 680px); min-height: 0;
  flex-direction: column; padding: 14px;
  border: 1px solid var(--reader-chrome-border);
  border-radius: 16px;
  color: var(--reader-chrome-text); background: var(--reader-chrome-bg);
  box-shadow: var(--app-shadow);
}
.reader-directory-header > div { display: grid; min-width: 0; gap: 3px; }
.reader-directory-header strong { overflow: hidden; color: var(--app-text); text-overflow: ellipsis; white-space: nowrap; }
.reader-directory-header small,
.reader-directory-header span { color: var(--app-muted); font-size: 12px; }
.reader-directory-list { display: grid; min-height: 0; flex: 1; gap: 6px; overflow: auto; overscroll-behavior: contain; }
.reader-directory-search {
  display: flex;
  min-height: 40px;
  align-items: center;
  gap: 8px;
  margin: 10px 0;
  padding: 0 10px;
  border: 1px solid var(--reader-chrome-border);
  border-radius: 10px;
  color: var(--app-muted);
  background: var(--app-surface);
}
.reader-directory-search input {
  width: 100%;
  min-width: 0;
  border: 0;
  outline: 0;
  color: var(--app-text);
  background: transparent;
  font-size: 13px;
}
.reader-directory-chapter {
  display: grid; width: 100%;
  min-height: 48px; grid-template-columns: 30px minmax(0, 1fr) auto;
  align-items: center; gap: 8px;
  padding: 7px 10px;
  border: 1px solid var(--reader-chrome-border);
  border-radius: 10px;
  color: var(--reader-chrome-text);
  background: var(--app-surface);
  text-align: left;
  cursor: pointer;
}
.reader-directory-chapter:hover:not(:disabled) {
  border-color: var(--app-accent);
  background: var(--app-accent-soft);
}
.reader-directory-chapter.current {
  border-color: var(--app-accent-soft);
  color: var(--app-accent-ink);
  background: var(--app-accent-soft);
}
.reader-directory-chapter:disabled {
  opacity: .55;
  cursor: default;
}
.reader-directory-number,
.reader-directory-current { color: var(--app-muted); font-size: 12px; }
.reader-directory-chapter.current .reader-directory-current { color: var(--app-accent-ink); }
.reader-directory-empty { margin: 0; padding: 16px 8px; color: var(--app-muted); text-align: center; }
.reader-settings-popover input:not([type="range"]):not([type="checkbox"]),
.reader-settings-popover select {
  min-height: 38px;
  padding: 0 10px;
  border: 1px solid var(--reader-chrome-border);
  border-radius: 9px;
  color: var(--app-text);
  background: var(--app-surface);
  font: inherit;
}
.reader-audio-choice-grid > button,
.reader-audio-sleep-choices > button,
.pdf-scale-control > div > button,
.comic-scale-control > div > button,
.comic-mode-control > button {
  min-height: 40px;
  padding: 8px 10px;
  border: 1px solid var(--reader-chrome-border);
  border-radius: 10px;
  color: var(--reader-chrome-text);
  background: var(--app-surface);
  cursor: pointer;
}
.reader-audio-choice-grid > button.selected,
.reader-audio-sleep-choices > button.selected,
.pdf-scale-control > div > button.selected,
.comic-scale-control > div > button.selected,
.comic-mode-control > button.selected {
  border-color: var(--app-accent);
  color: var(--app-accent-ink);
  background: var(--app-accent-soft);
}

.reader-audio-console {
  display: grid; width: min(820px, calc(100% - 40px));
  max-height: calc(100% - 112px); grid-template-columns: minmax(160px, 280px) minmax(0, 1fr);
  align-items: center; gap: 30px;
  margin: auto;
}
.reader-shell.audio-console-active .reader-content-frame { position: absolute; inset: 0; opacity: 0; pointer-events: none; }
.reader-audio-art { width: 100%; max-width: 280px; aspect-ratio: 1; object-fit: cover; }
.reader-audio-fallback { display: grid; place-items: center; }
.reader-audio-body, .reader-tts-panel { display: grid; min-width: 0; gap: 8px; }
.reader-audio-progress { grid-template-columns: auto minmax(0, 1fr) auto; margin-top: 20px; }
.reader-audio-playback, .reader-audio-utilities {
  display: flex; align-items: center;
  justify-content: center; gap: 12px;
  margin-top: 12px;
}
.reader-audio-main-play { min-width: 56px; min-height: 56px; }
.reader-audio-playback > button,
.reader-audio-utilities > button {
  display: grid;
  min-width: 40px;
  min-height: 40px;
  place-items: center;
  padding: 0 10px;
  border: 1px solid var(--reader-chrome-border);
  border-radius: 10px;
  color: var(--reader-chrome-text);
  background: var(--reader-chrome-bg);
  cursor: pointer;
}
.reader-audio-playback > .reader-audio-main-play {
  min-width: 56px;
  min-height: 56px;
  border-color: var(--app-accent);
  border-radius: 50%;
  color: #fff;
  background: var(--app-accent);
}
.reader-audio-playback > button:hover:not(:disabled),
.reader-audio-utilities > button:hover:not(:disabled) {
  border-color: var(--app-accent);
  color: var(--app-accent-ink);
  background: var(--app-accent-soft);
}
.reader-audio-playback > .reader-audio-main-play:hover:not(:disabled) {
  color: #fff;
  background: var(--app-accent-ink);
}
.reader-audio-choice-grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 7px; }
.reader-audio-sleep-choices { display: grid; grid-column: 1 / -1; }
.reader-audio-sleep-choices > button { display: flex; min-height: 48px; align-items: center; justify-content: space-between; }
.reader-choice-radio { width: 14px; height: 14px; flex: none; border: 1px solid var(--reader-chrome-border); border-radius: 50%; }
.reader-audio-sleep-choices > button.selected .reader-choice-radio { border: 4px solid var(--reader-accent); }

.reader-video-episodes { width: min(1120px, 100%); padding-top: 12px; }
.reader-video-episodes > header { display: flex; align-items: center; justify-content: space-between; gap: 10px; }
.reader-video-episodes nav { display: flex; gap: 8px; overflow-x: auto; }
.reader-video-controls {
  display: grid; grid-template-columns: 34px auto minmax(0, 1fr) auto;
  align-items: center; gap: 10px;
  margin-top: 12px;
}
.reader-video-extra-controls { display: flex; grid-column: 1 / -1; align-items: center; gap: 8px; }
.reader-video-extra-controls input, .reader-video-controls input { width: 100%; min-width: 0; }
.reader-video-play-toggle { display: grid; place-items: center; }
.reader-video-play-toggle,
.reader-video-fullscreen-toggle,
.reader-video-episode-button {
  min-height: 38px;
  padding: 0 11px;
  border: 1px solid var(--reader-chrome-border);
  border-radius: 10px;
  color: var(--reader-chrome-text);
  background: var(--reader-chrome-bg);
  cursor: pointer;
}
.reader-video-play-toggle { width: 40px; padding: 0; }
.reader-video-episodes nav > button {
  min-width: 36px;
  min-height: 36px;
  border: 1px solid var(--reader-chrome-border);
  border-radius: 9px;
  color: var(--reader-chrome-text);
  background: var(--reader-chrome-bg);
  cursor: pointer;
}
.reader-video-episodes nav > button.active {
  border-color: var(--app-accent);
  color: var(--app-accent-ink);
  background: var(--app-accent-soft);
}
.reader-video-play-toggle:hover:not(:disabled),
.reader-video-fullscreen-toggle:hover:not(:disabled),
.reader-video-episode-button:hover:not(:disabled),
.reader-video-episodes nav > button:hover:not(:disabled) {
  border-color: var(--app-accent);
  color: var(--app-accent-ink);
  background: var(--app-accent-soft);
}
.reader-video-time { min-width: 37px; }
.reader-video-rate-select { display: flex; align-items: center; gap: 5px; }

.pdf-scale-control, .comic-scale-control { display: grid; gap: 8px; }
.pdf-scale-control > div, .comic-scale-control > div { display: grid; grid-auto-flow: column; grid-auto-columns: 1fr; gap: 6px; }
.comic-mode-control { display: grid; }
.comic-mode-control > button {
  display: grid; grid-template-columns: minmax(0, 1fr) 16px;
  grid-template-rows: auto auto; align-items: center;
  gap: 3px 9px;
}
.comic-mode-control > button::after {
  content: ""; grid-column: 2;
  grid-row: 1 / 3; width: 14px;
  height: 14px; border: 1px solid var(--reader-chrome-border);
  border-radius: 50%;
}
.comic-mode-control > button.selected::after { border: 4px solid var(--reader-accent); }
.comic-brightness-control { display: grid; grid-template-columns: 60px minmax(0, 1fr) 50px; align-items: center; gap: 10px; }
.comic-brightness-control input { width: 100%; min-width: 0; }

#reader-chapter-jump { width: 76px; min-height: 36px; }
.reader-add-bookmark, .reader-dictionary-open { width: 100%; }
.reader-audio-rate-control > div, .reader-tts-row, .reader-current-bookmark, .reader-find-actions-row {
  display: flex; align-items: center;
  justify-content: space-between; gap: 8px;
}
.reader-tts-description { display: grid; gap: 4px; }
.reader-tts-panel select { width: 100%; min-width: 0; }
.reader-tts-rate input { flex: 1; min-width: 0; }
.reader-tts-continuous { display: flex; align-items: center; gap: 7px; }
.reader-tts-actions, .reader-find-buttons { display: flex; justify-content: flex-end; gap: 7px; }
.reader-tts-panel > p { margin: 0; }
.reader-find-control { grid-template-columns: minmax(0, 1fr); }
#reader-find-input { width: 100%; min-width: 0; }
.reader-audio-element { display: none; }

@media (max-width: 700px) {
  .reader-main { padding-right: 0; padding-left: 0; }
  .reader-audio-console { width: min(420px, calc(100% - 36px)); grid-template-columns: 1fr; gap: 17px; }
  .reader-audio-art { width: min(48vw, 210px); margin: 0 auto; }
  .reader-video-controls { grid-template-columns: 30px auto minmax(0, 1fr) auto; gap: 5px; }
}
@media (max-width: 640px) {
  .reader-toolbar-row { grid-template-columns: 36px minmax(0, 1fr) auto; gap: 5px; }
  .reader-header-actions .read-save-state { display: none; }
  .reader-settings-popover { right: 8px; max-height: 68dvh; }
  .reader-directory-panel { width: min(340px, 100%); height: min(68dvh, 680px); }
  .reader-comic-dock .reader-bottom-nav, .reader-pdf-dock .reader-bottom-nav { gap: 6px; }
}



.modal-backdrop.reader-dictionary-backdrop {
  z-index: 40;
  backdrop-filter: blur(3px);
}

.reader-bookmark-backdrop {
  backdrop-filter: blur(3px);
}

.dictionary-panel {
  display: flex;
  width: min(760px, 100%);
  max-height: min(88dvh, 900px);
  flex-direction: column;
  overflow: hidden;
  border: 1px solid var(--app-line);
  border-radius: 14px;
  background: var(--app-surface);
  box-shadow: 0 18px 46px rgb(30 33 45 / 16%);
}

.dictionary-panel > header {
  display: flex;
  min-height: 58px;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 8px 16px;
  border-bottom: 1px solid var(--app-line);
}

.dictionary-panel > header h2 {
  margin: 2px 0 0;
  font-size: 16px;
}

.dictionary-panel iframe {
  width: 100%;
  min-height: 0;
  flex: 1;
  border: 0;
  background: #fff;
}

.reader-dictionary-form {
  display: grid;
  gap: 10px;
  padding: 12px 16px;
  border-bottom: 1px solid var(--app-line);
}

.reader-dictionary-row {
  display: flex;
  align-items: center;
  gap: 8px;
}

.reader-dictionary-word,
.reader-dictionary-language {
  min-width: 0;
  flex: 1;
}

.reader-dictionary-language-label {
  white-space: nowrap;
}

.reader-dictionary-error {
  margin: 0;
  color: #a34f46;
  font-size: 13px;
}

.reader-dictionary-source {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 16px;
  color: var(--app-muted);
  font-size: 12px;
}

.reader-dictionary-status {
  position: relative;
  min-height: 110px;
}

@media (max-width: 860px) {
  .dictionary-panel {
    width: 100%;
    max-height: 94dvh;
  }
}
</style>

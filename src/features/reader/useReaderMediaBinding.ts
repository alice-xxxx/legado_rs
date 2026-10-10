import type { Ref } from "vue";
import { clamp } from "./displayHtml";
import type { useReaderMedia } from "./useReaderMedia";

export interface ReaderVideoQualityChoice {
  label: string;
  src: string;
  isHls: boolean;
  unavailableReason?: string;
  selected: boolean;
}

type ReaderMediaBindingOptions = {
  reader: {
    visible: Readonly<Ref<boolean>>;
    mediaChapter: Readonly<Ref<boolean>>;
    videoChapter: Readonly<Ref<boolean>>;
    chapterIndex: Readonly<Ref<number>>;
    timeMs: Ref<number>;
    durationMs: Ref<number>;
    playing: Ref<boolean>;
    videoPlaybackRate: Ref<number>;
    error: Ref<string>;
    progressDirty: Ref<boolean>;
    progressSaveState: Ref<"idle" | "saving" | "saved" | "error">;
    qualityChoices: Ref<ReaderVideoQualityChoice[]>;
    qualityIndex: Ref<number>;
    getLoadRevision: () => number;
    getMediaElement: () => HTMLMediaElement | null;
    setMediaElement: (media: HTMLMediaElement | null) => void;
    getAudioPlaybackRate: () => number;
    incrementProgressRevision: () => void;
  };
  actions: {
    changeChapter: (direction: -1 | 1) => Promise<void>;
    saveCurrentProgress: () => Promise<void>;
    isControlBlocked: () => boolean;
    updateAudioPlaybackRate: (rate: number) => void;
    updateVideoPlaybackRate: (rate: number) => void;
    notify: (message: string, kind?: "success" | "error") => void;
    errorText: (error: unknown) => string;
    formatMediaTime: (milliseconds: number) => string;
  };
  mediaPlayback: ReturnType<typeof useReaderMedia>;
};

/** 管理阅读器 iframe 中媒体元素的绑定、进度保存和清晰度切换。 */
export function useReaderMediaBinding(options: ReaderMediaBindingOptions) {
  let mediaEventHandlers: Array<[string, EventListener]> = [];
  let mediaRevision = 0;
  let mediaSourceRevision = 0;
  let restoreTargetMs: number | null = null;
  let saveTimer: ReturnType<typeof setTimeout> | null = null;
  let qualityResumeHandler: EventListener | null = null;

  function clearQualityResumeHandler(media: HTMLMediaElement): void {
    if (!qualityResumeHandler) return;
    media.removeEventListener("loadedmetadata", qualityResumeHandler);
    qualityResumeHandler = null;
  }

  function loadReaderMediaSource(
    media: HTMLMediaElement,
    source: string,
    isHls: boolean,
    bindingRevision: number,
    loadRevision: number,
  ): number {
    const sourceRevision = ++mediaSourceRevision;
    const isCurrent = () => options.reader.getMediaElement() === media
      && mediaRevision === bindingRevision
      && options.reader.getLoadRevision() === loadRevision
      && mediaSourceRevision === sourceRevision
      && options.reader.visible.value
      && options.reader.mediaChapter.value;
    options.mediaPlayback.loadSource(media, source, isHls, isCurrent);
    return sourceRevision;
  }

  function clearReaderMediaBinding(pause = true): void {
    options.mediaPlayback.clear();
    mediaSourceRevision += 1;
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = null;
    const media = options.reader.getMediaElement();
    if (media && qualityResumeHandler) clearQualityResumeHandler(media);
    else qualityResumeHandler = null;
    if (media) {
      for (const [eventName, handler] of mediaEventHandlers) media.removeEventListener(eventName, handler);
    }
    mediaEventHandlers = [];
    options.reader.setMediaElement(null);
    options.reader.playing.value = false;
    mediaRevision += 1;
    restoreTargetMs = null;
    options.reader.qualityChoices.value = [];
    options.reader.qualityIndex.value = 0;
    if (pause && media) {
      try {
        media.pause();
      } catch {
        // 嵌入式 WebView 卸载媒体元素时，元素可能已经失效。
      }
    }
  }

  function scheduleReaderMediaProgressSave(bindingRevision: number, loadRevision: number): void {
    if (saveTimer) return;
    saveTimer = setTimeout(() => {
      saveTimer = null;
      if (mediaRevision !== bindingRevision || options.reader.getLoadRevision() !== loadRevision
        || !options.reader.visible.value) return;
      void options.actions.saveCurrentProgress();
    }, 5000);
  }

  function syncReaderMediaPosition(
    media: HTMLMediaElement,
    bindingRevision: number,
    loadRevision: number,
    scheduleSave = true,
  ): void {
    if (options.reader.getMediaElement() !== media || mediaRevision !== bindingRevision
      || options.reader.getLoadRevision() !== loadRevision) return;
    const positionMs = Math.max(0, Math.round(media.currentTime * 1000));
    options.reader.durationMs.value = Number.isFinite(media.duration) ? Math.max(0, Math.round(media.duration * 1000)) : 0;
    if (restoreTargetMs != null) {
      if (Math.abs(positionMs - restoreTargetMs) <= 500) {
        restoreTargetMs = null;
        options.reader.timeMs.value = positionMs;
      }
      return;
    }
    if (positionMs === options.reader.timeMs.value) return;
    options.reader.timeMs.value = positionMs;
    options.reader.progressDirty.value = true;
    options.reader.progressSaveState.value = "idle";
    options.reader.incrementProgressRevision();
    if (scheduleSave) scheduleReaderMediaProgressSave(bindingRevision, loadRevision);
  }

  async function flushReaderMediaProgress(
    media = options.reader.getMediaElement(),
    bindingRevision = mediaRevision,
    loadRevision = options.reader.getLoadRevision(),
  ): Promise<void> {
    if (media && options.reader.getMediaElement() === media) {
      syncReaderMediaPosition(media, bindingRevision, loadRevision, false);
    }
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = null;
    await options.actions.saveCurrentProgress();
  }

  async function pauseReaderMediaAndSave(): Promise<boolean> {
    const media = options.reader.getMediaElement();
    if (!media || !options.reader.mediaChapter.value) return true;
    const bindingRevision = mediaRevision;
    const loadRevision = options.reader.getLoadRevision();
    clearQualityResumeHandler(media);
    if (!media.paused) media.pause();
    await flushReaderMediaProgress(media, bindingRevision, loadRevision);
    return options.reader.getMediaElement() === media && mediaRevision === bindingRevision
      && !options.reader.progressDirty.value;
  }

  function selectReaderVideoQuality(index: number): void {
    const video = options.reader.getMediaElement();
    const choice = options.reader.qualityChoices.value[index];
    if (!video || video.tagName.toLowerCase() !== "video" || !options.reader.videoChapter.value
      || options.actions.isControlBlocked() || !choice?.src || choice.unavailableReason
      || index === options.reader.qualityIndex.value) return;
    const previousIndex = options.reader.qualityIndex.value;
    const bindingRevision = mediaRevision;
    const loadRevision = options.reader.getLoadRevision();
    const currentTimeMs = Number.isFinite(video.currentTime)
      ? Math.max(0, Math.round(video.currentTime * 1000))
      : options.reader.timeMs.value;
    const wasPlaying = !video.paused && !video.ended;
    let sourceRevision = mediaSourceRevision + 1;
    const isCurrent = () => options.reader.getMediaElement() === video
      && mediaRevision === bindingRevision
      && options.reader.getLoadRevision() === loadRevision
      && options.reader.visible.value
      && options.reader.videoChapter.value;
    const canResumePlayback = () => isCurrent()
      && mediaSourceRevision === sourceRevision
      && !options.actions.isControlBlocked();

    clearQualityResumeHandler(video);
    options.reader.timeMs.value = currentTimeMs;
    restoreTargetMs = currentTimeMs;
    options.reader.qualityIndex.value = index;
    options.reader.error.value = "";
    try {
      video.pause();
      const resumeAfterMetadata: EventListener = () => {
        if (qualityResumeHandler === resumeAfterMetadata) qualityResumeHandler = null;
        if (!canResumePlayback() || !wasPlaying) return;
        try {
          void video.play().catch((error: unknown) => options.mediaPlayback.reportActionError(error, isCurrent, "播放"));
        } catch (error) {
          options.mediaPlayback.reportActionError(error, isCurrent, "播放");
        }
      };
      qualityResumeHandler = resumeAfterMetadata;
      video.addEventListener("loadedmetadata", resumeAfterMetadata, { once: true });
      sourceRevision = loadReaderMediaSource(video, choice.src, choice.isHls, bindingRevision, loadRevision);
    } catch (error) {
      mediaSourceRevision += 1;
      options.mediaPlayback.destroyHlsInstance();
      clearQualityResumeHandler(video);
      restoreTargetMs = null;
      options.reader.qualityIndex.value = previousIndex;
      options.reader.error.value = `切换视频清晰度失败：${options.actions.errorText(error)}`;
      options.actions.notify(options.reader.error.value, "error");
    }
  }

  function onReaderVideoQualityChange(event: Event): void {
    const select = event.currentTarget;
    if (select && (select as HTMLElement).tagName.toLowerCase() === "select") {
      selectReaderVideoQuality(Number((select as HTMLSelectElement).value));
    }
  }

  function bindReaderMediaElement(media: HTMLMediaElement): void {
    clearReaderMediaBinding(false);
    const bindingRevision = ++mediaRevision;
    const loadRevision = options.reader.getLoadRevision();
    options.reader.setMediaElement(media);
    options.reader.playing.value = !media.paused;
    const isCurrent = () => options.reader.getMediaElement() === media
      && mediaRevision === bindingRevision
      && options.reader.getLoadRevision() === loadRevision
      && options.reader.visible.value
      && options.reader.mediaChapter.value;

    const onLoadedMetadata = () => {
      if (!isCurrent()) return;
      options.reader.durationMs.value = Number.isFinite(media.duration) ? Math.max(0, Math.round(media.duration * 1000)) : 0;
      const desiredMs = Math.max(0, options.reader.timeMs.value);
      const targetMs = Number.isFinite(media.duration) ? Math.min(desiredMs, Math.round(media.duration * 1000)) : desiredMs;
      options.reader.timeMs.value = targetMs;
      if (Math.abs(Math.round(media.currentTime * 1000) - targetMs) > 250) {
        restoreTargetMs = targetMs;
        try {
          media.currentTime = targetMs / 1000;
        } catch {
          restoreTargetMs = null;
          options.reader.error.value = `无法跳转到上次的${options.reader.videoChapter.value ? "视频" : "音频"}位置，请使用媒体控制条手动定位。`;
        }
      }
      options.mediaPlayback.syncSession(media);
    };
    const onRateChange = () => {
      if (!isCurrent()) return;
      const rate = Math.round(clamp(media.playbackRate, 0.5, 3) * 4) / 4;
      if (options.reader.videoChapter.value) {
        options.reader.videoPlaybackRate.value = rate;
        options.actions.updateVideoPlaybackRate(rate);
      } else if (Math.abs(rate - options.reader.getAudioPlaybackRate()) > 0.001) {
        options.actions.updateAudioPlaybackRate(rate);
      }
      options.mediaPlayback.syncSession(media);
    };
    const onTimeUpdate = () => {
      if (isCurrent()) {
        syncReaderMediaPosition(media, bindingRevision, loadRevision);
        options.mediaPlayback.syncSession(media);
      }
    };
    const onPlay = () => {
      if (!isCurrent()) return;
      options.reader.playing.value = true;
      options.mediaPlayback.syncSession(media);
    };
    const onSeeked = () => {
      if (!isCurrent()) return;
      if (restoreTargetMs != null && Math.abs(Math.round(media.currentTime * 1000) - restoreTargetMs) <= 500) {
        options.reader.timeMs.value = restoreTargetMs;
        restoreTargetMs = null;
        options.mediaPlayback.syncSession(media);
        return;
      }
      restoreTargetMs = null;
      syncReaderMediaPosition(media, bindingRevision, loadRevision, false);
      options.mediaPlayback.syncSession(media);
      void flushReaderMediaProgress(media, bindingRevision, loadRevision);
    };
    const onPause = () => {
      if (isCurrent()) {
        options.reader.playing.value = false;
        options.mediaPlayback.syncSession(media);
        void flushReaderMediaProgress(media, bindingRevision, loadRevision);
      }
    };
    const onEnded = () => {
      if (!isCurrent()) return;
      options.reader.playing.value = false;
      options.mediaPlayback.syncSession(media);
      void (async () => {
        await flushReaderMediaProgress(media, bindingRevision, loadRevision);
        if (!isCurrent()) return;
        const finishedChapter = options.reader.chapterIndex.value;
        await options.actions.changeChapter(1);
        if (options.reader.chapterIndex.value === finishedChapter + 1 && options.reader.visible.value) {
          options.actions.notify(`本章${options.reader.videoChapter.value ? "视频" : "音频"}已结束，下一章已打开。点击播放继续。`);
        }
      })();
    };
    const onError = () => {
      if (!isCurrent() || media.error?.code === MediaError.MEDIA_ERR_ABORTED) return;
      if (options.reader.error.value.startsWith("HLS") || options.reader.error.value.startsWith("此平台不支持 HLS")) {
        options.mediaPlayback.syncSession(media);
        return;
      }
      clearQualityResumeHandler(media);
      options.reader.playing.value = false;
      options.mediaPlayback.syncSession(media);
      const detail = media.error?.code === MediaError.MEDIA_ERR_NETWORK
        ? "网络读取失败"
        : media.error?.code === MediaError.MEDIA_ERR_DECODE
          ? `${options.reader.videoChapter.value ? "视频" : "音频"}解码失败`
          : media.error?.code === MediaError.MEDIA_ERR_SRC_NOT_SUPPORTED
            ? `${options.reader.videoChapter.value ? "视频" : "音频"}格式或资源不可用`
            : `${options.reader.videoChapter.value ? "视频" : "音频"}播放失败`;
      options.reader.error.value = `${detail}。可以点击“重新加载”重试获取当前媒体资源。`;
      options.actions.notify(options.reader.error.value, "error");
    };
    mediaEventHandlers = [
      ["loadedmetadata", onLoadedMetadata],
      ["ratechange", onRateChange],
      ["play", onPlay],
      ["timeupdate", onTimeUpdate],
      ["seeked", onSeeked],
      ["pause", onPause],
      ["ended", onEnded],
      ["error", onError],
    ];
    for (const [eventName, handler] of mediaEventHandlers) media.addEventListener(eventName, handler);
    options.mediaPlayback.setSession(media, isCurrent);
    try {
      const rate = options.reader.videoChapter.value
        ? options.reader.videoPlaybackRate.value
        : options.reader.getAudioPlaybackRate();
      if (Math.abs(media.playbackRate - rate) > 0.001) media.playbackRate = rate;
    } catch {
      options.reader.error.value = `当前${options.reader.videoChapter.value ? "视频" : "音频"}播放器无法调整播放速度。`;
    }
    if (media.readyState >= HTMLMediaElement.HAVE_METADATA) onLoadedMetadata();
  }

  return {
    bindReaderMediaElement,
    clearQualityResumeHandler,
    clearReaderMediaBinding,
    flushReaderMediaProgress,
    getBindingRevision: () => mediaRevision,
    invalidateSource: () => { mediaSourceRevision += 1; },
    loadReaderMediaSource,
    onReaderVideoQualityChange,
    pauseReaderMediaAndSave,
    scheduleReaderMediaProgressSave,
    selectReaderVideoQuality,
    syncReaderMediaPosition,
  };
}

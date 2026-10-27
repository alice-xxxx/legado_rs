import { readingBook } from "./readerBookState";
import type { Ref } from "vue";
import type Hls from "hls.js";
import { openMediaSession } from "./mediaSession";

type ReaderMediaContext = {
  visible: Readonly<Ref<boolean>>;
  mediaChapter: Readonly<Ref<boolean>>;
  videoChapter: Readonly<Ref<boolean>>;
  chapterTitle: Readonly<Ref<string>>;
  getCurrentElement: () => HTMLMediaElement | null;
  isControlBlocked: () => boolean;
  error: Ref<string>;
};

type UseReaderMediaOptions = {
  reader: ReaderMediaContext;
  changeChapter: (direction: -1 | 1) => Promise<void>;
  pauseAndSave: () => Promise<boolean>;
  clearQualityResumeHandler: (media: HTMLMediaElement) => void;
  notify: (message: string, kind?: "success" | "error") => void;
  errorText: (error: unknown) => string;
  getSeekSeconds: () => number;
};

/** 管理系统媒体会话和 HLS 资源；播放器事件与阅读进度仍由阅读器页面管理。 */
export function useReaderMedia(options: UseReaderMediaOptions) {
  let mediaSession: ReturnType<typeof openMediaSession> | undefined;
  let chapterChangePending = false;
  let hlsInstance: Hls | null = null;
  let hlsLoadRevision = 0;

  function clearSession(): void {
    mediaSession?.close();
    mediaSession = undefined;
  }

  function syncSession(audio = options.reader.getCurrentElement()): void {
    if (audio) mediaSession?.sync(audio);
  }

  function reportActionError(error: unknown, isCurrent: () => boolean, action: string): void {
    if (!isCurrent()) return;
    const mediaName = options.reader.videoChapter.value ? "视频" : "音频";
    options.reader.error.value = mediaName + action + "失败：" + options.errorText(error);
    options.notify(options.reader.error.value, "error");
  }

  function setSession(audio: HTMLMediaElement, isCurrent: () => boolean): void {
    const canChangeMedia = () => isCurrent() && !options.reader.isControlBlocked();
    const seekTo = (seconds: number): void => {
      if (!canChangeMedia() || !Number.isFinite(seconds)) return;
      const target = Math.max(0, Number.isFinite(audio.duration) ? Math.min(seconds, audio.duration) : seconds);
      try {
        audio.currentTime = target;
      } catch (error) {
        reportActionError(error, isCurrent, "定位");
      }
    };
    const changeMediaChapter = (direction: -1 | 1): void => {
      if (!canChangeMedia() || chapterChangePending) return;
      chapterChangePending = true;
      void options.changeChapter(direction)
        .catch((error: unknown) => reportActionError(error, isCurrent, "切换章节"))
        .finally(() => {
          chapterChangePending = false;
        });
    };

    const book = readingBook.value;
    mediaSession = openMediaSession({
      title: options.reader.chapterTitle.value,
      artist: book?.author ?? "",
      album: book?.title ?? "",
      artwork: book?.coverSrc ? [{ src: book.coverSrc }] : [],
    }, {
      play: () => { if (canChangeMedia() && audio.paused) return audio.play(); },
      pause: () => { if (isCurrent()) return options.pauseAndSave(); },
      stop: () => { if (isCurrent()) return options.pauseAndSave(); },
      seekto: details => { if (details.seekTime !== undefined) seekTo(details.seekTime); },
      seekbackward: details => seekTo(audio.currentTime - (details.seekOffset ?? options.getSeekSeconds())),
      seekforward: details => seekTo(audio.currentTime + (details.seekOffset ?? options.getSeekSeconds())),
      previoustrack: () => changeMediaChapter(-1),
      nexttrack: () => changeMediaChapter(1),
    }, error => reportActionError(error, isCurrent, "控制"));
    syncSession(audio);
  }

  function destroyHlsInstance(): void {
    ++hlsLoadRevision;
    const hls = hlsInstance;
    hlsInstance = null;
    if (hls) {
      try {
        hls.destroy();
      } catch {
        // 阅读界面卸载时媒体元素可能已失效。
      }
    }
  }

  function loadSource(media: HTMLMediaElement, source: string, isHls: boolean, isCurrent: () => boolean): void {
    destroyHlsInstance();
    if (!source) throw new Error("媒体资源地址为空。");
    options.reader.error.value = "";
    if (!isHls) {
      media.src = source;
      media.load();
      return;
    }
    const supportsNativeHls = !!media.canPlayType("application/vnd.apple.mpegurl")
      || !!media.canPlayType("application/x-mpegURL");
    if (supportsNativeHls) {
      media.src = source;
      media.load();
      return;
    }
    // hls.js is a substantial dependency; only load it for non-native HLS.
    // Most books, audio, PDFs and Safari's native HLS never need this chunk.
    const revision = hlsLoadRevision;
    media.removeAttribute("src");
    media.querySelectorAll("source").forEach((sourceElement) => sourceElement.removeAttribute("src"));
    void import("hls.js").then(({ default: Hls }) => {
      if (revision !== hlsLoadRevision || !isCurrent()) return;
      if (!Hls.isSupported()) {
        options.reader.error.value = "此平台不支持 HLS 播放（缺少原生支持或 MSE 解码能力）。";
        media.pause();
        options.clearQualityResumeHandler(media);
        options.notify(options.reader.error.value, "error");
        return;
      }
      const hls = new Hls({ enableWorker: false });
      hlsInstance = hls;
      hls.on(Hls.Events.ERROR, (_event, data) => {
        if (!data.fatal || !isCurrent() || revision !== hlsLoadRevision) return;
        const failure = data.type === "networkError"
          ? "HLS网络读取失败"
          : data.type === "mediaError"
            ? "HLS格式或媒体解码失败"
            : "HLS播放失败";
        options.reader.error.value = failure + "。可以点击“重新加载”重试当前媒体资源。";
        media.pause();
        options.clearQualityResumeHandler(media);
        options.notify(options.reader.error.value, "error");
        if (hlsInstance === hls) destroyHlsInstance();
      });
      hls.attachMedia(media);
      hls.loadSource(source);
    }).catch((error: unknown) => {
      if (revision !== hlsLoadRevision || !isCurrent()) return;
      options.reader.error.value = "HLS 播放器加载失败：" + options.errorText(error);
      options.clearQualityResumeHandler(media);
      options.notify(options.reader.error.value, "error");
    });
  }

  function clear(): void {
    clearSession();
    destroyHlsInstance();
  }

  return {
    clear,
    syncSession,
    setSession,
    reportActionError,
    destroyHlsInstance,
    loadSource,
  };
}

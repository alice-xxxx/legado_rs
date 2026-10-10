import type { Ref } from "vue";
import type Hls from "hls.js";
import { clamp } from "./displayHtml";
import type { BookResource } from "../../api/types";

type ReaderMediaContext = {
  visible: Readonly<Ref<boolean>>;
  mediaChapter: Readonly<Ref<boolean>>;
  videoChapter: Readonly<Ref<boolean>>;
  book: Readonly<Ref<BookResource | null>>;
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
  let mediaSession: MediaSession | null = null;
  let mediaSessionActions: MediaSessionAction[] = [];
  let chapterChangePending = false;
  let hlsInstance: Hls | null = null;
  let hlsLoadRevision = 0;

  function clearSession(): void {
    const session = mediaSession;
    if (!session) return;
    for (const action of mediaSessionActions) {
      try {
        session.setActionHandler(action, null);
      } catch {
        // 某些 WebView 不接受移除已不再支持的 action。
      }
    }
    mediaSessionActions = [];
    try {
      session.metadata = null;
    } catch {
      // 部分 Media Session 实现不提供 metadata。
    }
    try {
      session.playbackState = "none";
    } catch {
      // 部分 Media Session 实现不提供 playbackState。
    }
    try {
      session.setPositionState();
    } catch {
      // 部分 Media Session 实现不支持 position state。
    }
    mediaSession = null;
  }

  function syncSession(audio = options.reader.getCurrentElement()): void {
    const session = mediaSession;
    if (!session || !audio || !options.reader.visible.value || !options.reader.mediaChapter.value) return;
    try {
      session.playbackState = audio.paused || audio.ended ? "paused" : "playing";
    } catch {
      // 播放状态是可选能力。
    }
    if (typeof session.setPositionState !== "function" || !Number.isFinite(audio.duration) || audio.duration <= 0) return;
    try {
      session.setPositionState({
        duration: audio.duration,
        playbackRate: audio.playbackRate,
        position: clamp(audio.currentTime, 0, audio.duration),
      });
    } catch {
      // 浏览器可能拒绝过期或不支持的位置更新。
    }
  }

  function reportActionError(error: unknown, isCurrent: () => boolean, action: string): void {
    if (!isCurrent()) return;
    const mediaName = options.reader.videoChapter.value ? "视频" : "音频";
    options.reader.error.value = mediaName + action + "失败：" + options.errorText(error);
    options.notify(options.reader.error.value, "error");
  }

  function setSession(audio: HTMLMediaElement, isCurrent: () => boolean): void {
    if (typeof navigator === "undefined") return;
    let session: MediaSession | undefined;
    try {
      session = navigator.mediaSession;
    } catch {
      return;
    }
    if (!session || typeof session.setActionHandler !== "function") return;
    mediaSession = session;

    if (typeof MediaMetadata !== "undefined") {
      try {
        session.metadata = new MediaMetadata({
          title: options.reader.chapterTitle.value,
          artist: options.reader.book.value?.author ?? "",
          album: options.reader.book.value?.title ?? "",
        });
      } catch {
        // 有 action handler 的平台也可能不支持 metadata。
      }
    }

    const register = (action: MediaSessionAction, handler: MediaSessionActionHandler): void => {
      try {
        session!.setActionHandler(action, handler);
        mediaSessionActions.push(action);
      } catch {
        // 不同浏览器支持的媒体 action 不同。
      }
    };
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

    register("play", () => {
      if (!canChangeMedia() || !audio.paused) return;
      try {
        void audio.play().catch((error: unknown) => reportActionError(error, isCurrent, "播放"));
      } catch (error) {
        reportActionError(error, isCurrent, "播放");
      }
    });
    register("pause", () => {
      if (isCurrent()) void options.pauseAndSave();
    });
    register("seekto", (details) => {
      if (Number.isFinite(details.seekTime)) seekTo(details.seekTime!);
    });
    register("seekbackward", (details) => {
      if (isCurrent()) seekTo(audio.currentTime - (details.seekOffset ?? options.getSeekSeconds()));
    });
    register("seekforward", (details) => {
      if (isCurrent()) seekTo(audio.currentTime + (details.seekOffset ?? options.getSeekSeconds()));
    });
    register("previoustrack", () => changeMediaChapter(-1));
    register("nexttrack", () => changeMediaChapter(1));
    syncSession(audio);
  }

  function isHlsMediaSource(element: Element): boolean {
    return element.getAttribute("data-legado-media-format")?.trim().toLowerCase() === "hls";
  }

  function destroyHlsInstance(): void {
    ++hlsLoadRevision;
    const hls = hlsInstance;
    hlsInstance = null;
    if (hls) {
      try {
        hls.destroy();
      } catch {
        // iframe 卸载时媒体元素可能已失效。
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
    isHlsMediaSource,
    destroyHlsInstance,
    loadSource,
  };
}

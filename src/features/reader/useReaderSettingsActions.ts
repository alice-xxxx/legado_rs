import type { Ref } from "vue";
import { pickReaderBackgroundImage } from "../../api/settings";
import { type AppSettingsResource, type ReaderSettings } from "../../api/types";
import { clamp } from "./displayHtml";

interface ReaderSettingsActionOptions {
  settings: Ref<AppSettingsResource>;
  readerBackgroundBusy: Ref<boolean>;
  readerBackgroundError: Ref<string>;
  readerMediaError: Ref<string>;
  readerMediaElement: () => HTMLMediaElement | null;
  readingVideoChapter: Readonly<Ref<boolean>>;
  backupBusy: () => boolean;
  recoveryRequired: Readonly<Ref<boolean>>;
  rebuildReaderDisplay: () => void;
  notify: (message: string, kind?: "success" | "error") => void;
  errorText: (error: unknown) => string;
}

/** 集中处理阅读器外观、阅读设置持久化和背景图片更新。 */
export function useReaderSettingsActions(options: ReaderSettingsActionOptions) {
  function setReaderTheme(theme: ReaderSettings["theme"]): void {
    const colors: Record<string, { textColor: string; backgroundColor: string }> = {
      paper: { textColor: "#3f3b34", backgroundColor: "#f7f3e9" },
      sepia: { textColor: "#4e4131", backgroundColor: "#eee1c8" },
      dark: { textColor: "#d1d0cb", backgroundColor: "#17191c" },
      system: { textColor: "#252525", backgroundColor: "#ffffff" },
    };
    Object.assign(options.settings.value.reader, { theme, ...colors[theme] });
    options.rebuildReaderDisplay();
  }

  function applyAppearanceSettings(reader: ReaderSettings): void {
    const previous = { ...options.settings.value.reader };
    Object.assign(options.settings.value.reader, reader);
    // A playback preference is not a text-layout change. Avoid rebuilding the
    // reader iframe (and disrupting active media) for rate-only updates.
    const displayKeys = [
      "fontSizePx", "lineHeight", "fontFamily", "textColor", "backgroundColor",
      "backgroundImageSrc", "backgroundImageOpacity", "textAlign", "theme", "verticalScroll",
    ] as const satisfies ReadonlyArray<keyof ReaderSettings>;
    if (displayKeys.some((key) => previous[key] !== reader[key])) options.rebuildReaderDisplay();
    const media = options.readerMediaElement();
    const activeRate = options.readingVideoChapter.value ? reader.videoPlaybackRate : reader.audioPlaybackRate;
    const previousRate = options.readingVideoChapter.value ? previous.videoPlaybackRate : previous.audioPlaybackRate;
    if (media && activeRate !== previousRate && Math.abs(media.playbackRate - activeRate) > 0.001) {
      try { media.playbackRate = activeRate; }
      catch {
        options.readerMediaError.value = options.readingVideoChapter.value
          ? "当前视频播放器无法调整播放速度。" : "当前音频播放器无法调整播放速度。";
      }
    }
  }

  function updateReaderSetting<K extends keyof ReaderSettings>(key: K, value: ReaderSettings[K]): void {
    const valueToStore = key === "audioPlaybackRate" || key === "videoPlaybackRate"
      ? Math.round(clamp(Number(value), 0.5, 3) * 4) / 4 as ReaderSettings[K]
      : value;
    options.settings.value.reader[key] = valueToStore;
    const media = options.readerMediaElement();
    if ((key === "audioPlaybackRate" && !options.readingVideoChapter.value
      || key === "videoPlaybackRate" && options.readingVideoChapter.value) && media) {
      const rate = options.readingVideoChapter.value
        ? options.settings.value.reader.videoPlaybackRate : options.settings.value.reader.audioPlaybackRate;
      try {
        if (Math.abs(media.playbackRate - rate) > 0.001) media.playbackRate = rate;
      } catch {
        options.readerMediaError.value = options.readingVideoChapter.value
          ? "当前视频播放器无法调整播放速度。" : "当前音频播放器无法调整播放速度。";
      }
    }
    if (key !== "ttsRate" && key !== "ttsVoiceURI" && key !== "ttsContinueAcrossChapters"
      && key !== "ttsStartPosition" && key !== "audioPlaybackRate" && key !== "videoPlaybackRate") options.rebuildReaderDisplay();
  }

  function updateSourceHttpTimeout(seconds: number): void {
    options.settings.value.sourceHttpTimeoutSeconds = seconds;
  }

  async function chooseReaderBackgroundImage(): Promise<void> {
    if (options.readerBackgroundBusy.value || options.backupBusy()
      || options.recoveryRequired.value) return;
    options.readerBackgroundBusy.value = true;
    options.readerBackgroundError.value = "";
    try {
      const result = await pickReaderBackgroundImage();
      if (result.cancelled) return;
      options.settings.value.reader.backgroundImageSrc = result.image.src;
      options.notify("阅读背景图片已保存。");
    } catch (error) {
      options.readerBackgroundError.value = options.errorText(error);
      options.notify(`设置阅读背景图片失败：${options.readerBackgroundError.value}`, "error");
    } finally {
      options.readerBackgroundBusy.value = false;
    }
  }

  function clearReaderBackground(): void {
    if (options.readerBackgroundBusy.value || options.backupBusy() || options.recoveryRequired.value) return;
    delete options.settings.value.reader.backgroundImageSrc;
    options.readerBackgroundError.value = "";
    options.notify("阅读背景图片已清除。");
  }

  return {
    setReaderTheme,
    applyAppearanceSettings,
    updateReaderSetting,
    updateSourceHttpTimeout,
    chooseReaderBackgroundImage,
    clearReaderBackground,
  };
}

import type { Ref } from "vue";
import { clearReaderBackgroundImage, pickReaderBackgroundImage, saveSettings } from "../../api/settings";
import { readResource } from "../../api/resources";
import { type AppSettingsResource, type ReaderSettings } from "../../api/types";
import { clamp } from "./displayHtml";

interface ReaderSettingsActionOptions {
  settings: Ref<AppSettingsResource>;
  saveSettingsBusy: Ref<boolean>;
  readerBackgroundBusy: Ref<boolean>;
  readerBackgroundError: Ref<string>;
  pendingSettingsSave: Ref<ReturnType<typeof setTimeout> | null>;
  readerMediaError: Ref<string>;
  readerMediaElement: () => HTMLMediaElement | null;
  readingVideoChapter: Readonly<Ref<boolean>>;
  backupBusy: () => boolean;
  recoveryRequired: Readonly<Ref<boolean>>;
  deferSettingsSaveDuringRestore: () => boolean;
  normalizeSettings: (value: AppSettingsResource) => AppSettingsResource;
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
    options.settings.value = {
      ...options.settings.value,
      reader: { ...options.settings.value.reader, theme, ...colors[theme] },
    };
    options.rebuildReaderDisplay();
    scheduleSettingsSave();
  }

  function applyAppearanceSettings(reader: ReaderSettings): void {
    const previous = options.settings.value.reader;
    options.settings.value = { ...options.settings.value, reader };
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
    scheduleSettingsSave();
  }

  function updateReaderSetting<K extends keyof ReaderSettings>(key: K, value: ReaderSettings[K]): void {
    const valueToStore = key === "audioPlaybackRate" || key === "videoPlaybackRate"
      ? Math.round(clamp(Number(value), 0.5, 3) * 4) / 4 as ReaderSettings[K]
      : value;
    options.settings.value = {
      ...options.settings.value,
      reader: { ...options.settings.value.reader, [key]: valueToStore },
    };
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
    scheduleSettingsSave();
  }

  function updateSourceHttpTimeout(seconds: number): void {
    options.settings.value = options.normalizeSettings({ ...options.settings.value, sourceHttpTimeoutSeconds: seconds });
    scheduleSettingsSave();
  }

  function scheduleSettingsSave(): void {
    if (options.deferSettingsSaveDuringRestore()) return;
    if (options.pendingSettingsSave.value) clearTimeout(options.pendingSettingsSave.value);
    options.pendingSettingsSave.value = setTimeout(() => void persistSettings(), 450);
  }

  async function persistSettings(): Promise<boolean> {
    if (options.pendingSettingsSave.value) clearTimeout(options.pendingSettingsSave.value);
    options.pendingSettingsSave.value = null;
    options.saveSettingsBusy.value = true;
    try {
      const normalized = options.normalizeSettings(options.settings.value);
      // 背景图片源由图片选择器单独保存，避免写入当前运行时临时地址。
      delete normalized.reader.backgroundImageSrc;
      const descriptor = await saveSettings(normalized);
      options.settings.value = options.normalizeSettings(await readResource<AppSettingsResource>(descriptor));
      options.notify("阅读设置已保存。");
      return true;
    } catch (error) {
      options.notify(`阅读设置保存失败：${options.errorText(error)}`, "error");
      return false;
    } finally {
      options.saveSettingsBusy.value = false;
    }
  }

  async function flushPendingSettingsBeforeBackgroundChange(): Promise<void> {
    if (!options.pendingSettingsSave.value) return;
    clearTimeout(options.pendingSettingsSave.value);
    options.pendingSettingsSave.value = null;
    if (!(await persistSettings())) throw new Error("阅读设置保存失败，请重试后再更改背景图片。");
  }

  async function chooseReaderBackgroundImage(): Promise<void> {
    if (options.readerBackgroundBusy.value || options.saveSettingsBusy.value || options.backupBusy()
      || options.recoveryRequired.value) return;
    options.readerBackgroundBusy.value = true;
    options.readerBackgroundError.value = "";
    try {
      await flushPendingSettingsBeforeBackgroundChange();
      const result = await pickReaderBackgroundImage();
      if (result.cancelled) return;
      options.settings.value = options.normalizeSettings(await readResource<AppSettingsResource>(result.settings));
      options.rebuildReaderDisplay();
      options.notify("阅读背景图片已保存。");
    } catch (error) {
      options.readerBackgroundError.value = options.errorText(error);
      options.notify(`设置阅读背景图片失败：${options.readerBackgroundError.value}`, "error");
    } finally {
      options.readerBackgroundBusy.value = false;
    }
  }

  async function clearReaderBackground(): Promise<void> {
    if (options.readerBackgroundBusy.value || options.saveSettingsBusy.value || options.backupBusy()
      || options.recoveryRequired.value) return;
    options.readerBackgroundBusy.value = true;
    options.readerBackgroundError.value = "";
    try {
      await flushPendingSettingsBeforeBackgroundChange();
      const descriptor = await clearReaderBackgroundImage();
      options.settings.value = options.normalizeSettings(await readResource<AppSettingsResource>(descriptor));
      options.rebuildReaderDisplay();
      options.notify("阅读背景图片已清除。");
    } catch (error) {
      options.readerBackgroundError.value = options.errorText(error);
      options.notify(`清除阅读背景图片失败：${options.readerBackgroundError.value}`, "error");
    } finally {
      options.readerBackgroundBusy.value = false;
    }
  }

  return {
    setReaderTheme,
    applyAppearanceSettings,
    updateReaderSetting,
    updateSourceHttpTimeout,
    scheduleSettingsSave,
    persistSettings,
    chooseReaderBackgroundImage,
    clearReaderBackground,
  };
}

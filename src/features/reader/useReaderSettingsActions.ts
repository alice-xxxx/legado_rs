import type { Ref } from "vue";
import { pickReaderBackgroundImage } from "../../api/settings";
import { type AppSettingsResource, type ReaderSettings } from "../../api/types";
import { clamp } from "./readerContent";

interface ReaderSettingsActionOptions {
  settings: Ref<AppSettingsResource>;
  readerBackgroundBusy: Ref<boolean>;
  readerBackgroundError: Ref<string>;
  backupBusy: () => boolean;
  recoveryRequired: Readonly<Ref<boolean>>;
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
  }

  function applyAppearanceSettings(reader: ReaderSettings): void { Object.assign(options.settings.value.reader, reader); }
  function updateReaderSetting<K extends keyof ReaderSettings>(key: K, value: ReaderSettings[K]): void {
    options.settings.value.reader[key] = key === "audioPlaybackRate" || key === "videoPlaybackRate"
      ? Math.round(clamp(Number(value), 0.5, 3) * 4) / 4 as ReaderSettings[K] : value;
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

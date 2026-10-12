import type { ReaderSettings } from "../../api/types";
import { settings } from "../settings/settingsStore";
import { clamp } from "./readerContent";

/** 更新全局阅读器设置，持久化由 settingsStore 负责。 */
export function useReaderSettingsActions() {
  function updateReaderSetting<K extends keyof ReaderSettings>(key: K, value: ReaderSettings[K]): void {
    settings.value.reader[key] = key === "audioPlaybackRate" || key === "videoPlaybackRate"
      ? Math.round(clamp(Number(value), 0.5, 3) * 4) / 4 as ReaderSettings[K] : value;
  }

  return { updateReaderSetting };
}

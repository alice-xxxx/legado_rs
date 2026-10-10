import type { AppSettingsResource, ReaderSettings } from "../../api/types";
import { clamp } from "../reader/displayHtml";

/** 提供首次启动和缺省字段使用的统一设置值。 */
export const defaultSettings: AppSettingsResource = {
  schemaVersion: 1,
  sourceHttpTimeoutSeconds: 15,
  reader: {
    fontSizePx: 19,
    lineHeight: 1.8,
    fontFamily: "serif",
    textColor: "#3f3b34",
    backgroundColor: "#f7f3e9",
    backgroundImageOpacity: 20,
    textAlign: "justify",
    theme: "paper",
    preloadCount: 5,
    verticalScroll: false,
    audioPlaybackRate: 1,
    audioSkipSeconds: 15,
    videoPlaybackRate: 1,
    ttsEngine: "system",
    ttsRate: 1,
    ttsVoiceURI: "",
    httpTtsConfigId: null,
    ttsContinueAcrossChapters: true,
    ttsStartPosition: "current",
    replacements: [],
  },
};

/** 规范化旧配置和不完整数据，避免无效数值传入各设置页面。 */
export function normalizeSettings(value: AppSettingsResource): AppSettingsResource {
  const reader = value?.reader ?? {};
  const themeValue = typeof reader.theme === "string" ? reader.theme : defaultSettings.reader.theme;
  const theme: ReaderSettings["theme"] = themeValue === "light" ? "system"
    : ["paper", "sepia", "dark", "system"].includes(themeValue) ? themeValue as ReaderSettings["theme"] : "paper";
  const numeric = (candidate: unknown, fallback: number): number => Number(candidate ?? fallback);
  const lastBackupAtMs = value?.lastBackupAtMs;
  const timeout = numeric(value?.sourceHttpTimeoutSeconds, defaultSettings.sourceHttpTimeoutSeconds);

  return {
    ...defaultSettings,
    ...value,
    lastBackupAtMs: typeof lastBackupAtMs === "number" && Number.isFinite(lastBackupAtMs) && lastBackupAtMs > 0 ? lastBackupAtMs : null,
    sourceHttpTimeoutSeconds: [15, 30, 60, 120].includes(timeout) ? timeout : defaultSettings.sourceHttpTimeoutSeconds,
    reader: {
      ...defaultSettings.reader,
      ...reader,
      fontSizePx: clamp(numeric(reader.fontSizePx ?? (reader as Partial<ReaderSettings> & { fontSize?: number }).fontSize, defaultSettings.reader.fontSizePx), 12, 36),
      lineHeight: clamp(numeric(reader.lineHeight, defaultSettings.reader.lineHeight), 1.2, 2.8),
      preloadCount: clamp(numeric(reader.preloadCount, defaultSettings.reader.preloadCount), 1, 20),
      verticalScroll: typeof reader.verticalScroll === "boolean" ? reader.verticalScroll : defaultSettings.reader.verticalScroll,
      audioPlaybackRate: Math.round(clamp(numeric(reader.audioPlaybackRate, defaultSettings.reader.audioPlaybackRate), 0.5, 3) * 4) / 4,
      videoPlaybackRate: Math.round(clamp(numeric(reader.videoPlaybackRate, defaultSettings.reader.videoPlaybackRate), 0.5, 3) * 4) / 4,
      audioSkipSeconds: [10, 15, 30].includes(numeric(reader.audioSkipSeconds, defaultSettings.reader.audioSkipSeconds))
        ? numeric(reader.audioSkipSeconds, defaultSettings.reader.audioSkipSeconds) : 15,
      // 旧设置曾把两种引擎并列显示；若已配置 HTTP 源则优先延续旧的远程朗读入口。
      ttsEngine: reader.ttsEngine === "http" || reader.ttsEngine === "system"
        ? reader.ttsEngine
        : typeof reader.httpTtsConfigId === "string" && reader.httpTtsConfigId.length > 0 ? "http" : "system",
      ttsRate: clamp(numeric(reader.ttsRate, defaultSettings.reader.ttsRate), 0.5, 5),
      ttsVoiceURI: typeof reader.ttsVoiceURI === "string" ? reader.ttsVoiceURI : defaultSettings.reader.ttsVoiceURI,
      httpTtsConfigId: typeof reader.httpTtsConfigId === "string" && reader.httpTtsConfigId.length <= 128 ? reader.httpTtsConfigId : null,
      ttsContinueAcrossChapters: typeof reader.ttsContinueAcrossChapters === "boolean"
        ? reader.ttsContinueAcrossChapters : defaultSettings.reader.ttsContinueAcrossChapters,
      ttsStartPosition: reader.ttsStartPosition === "chapter" || reader.ttsStartPosition === "current"
        ? reader.ttsStartPosition : defaultSettings.reader.ttsStartPosition,
      backgroundImageSrc: typeof reader.backgroundImageSrc === "string" ? reader.backgroundImageSrc : undefined,
      backgroundImageOpacity: clamp(numeric(reader.backgroundImageOpacity, defaultSettings.reader.backgroundImageOpacity), 0, 100),
      theme,
      replacements: [],
    },
  };
}

import type { AppSettingsResource } from "../../api/types";

/** 提供首次启动和缺省字段使用的统一设置值。 */
export const defaultSettings: AppSettingsResource = {
  schemaVersion: 1,
  theme: "system",
  sourceHttpTimeoutSeconds: 15,
  home: { tabs: [{ id: "tab-home", title: "主页", sortOrder: 0, sections: [] }] },
  reader: {
    fontSizePx: 19,
    lineHeight: 1.8,
    fontFamily: "serif",
    backgroundImageOpacity: 20,
    textAlign: "justify",
    themes: { light: "paper", dark: "dark" },
    preloadCount: 5,
    readingMode: "cover",
    audioPlaybackRate: 1,
    audioSkipSeconds: 15,
    videoPlaybackRate: 1,
    ttsEngine: "system",
    ttsRate: 1,
    ttsVoiceURI: "",
    httpTtsConfigId: null,
    ttsContinueAcrossChapters: true,
    ttsStartPosition: "current",
  },
};

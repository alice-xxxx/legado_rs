import { computed, ref, watch } from "vue";
import materialColors from "konsta/material-colors";
import type { AppTheme, ReadingTheme, UiStyle } from "../../api/types";
import { settings } from "./settingsStore";

export const appThemes: ReadonlyArray<readonly [AppTheme, string]> = [
  ["light", "白天"], ["dark", "夜间"], ["system", "跟随系统"],
];
export const uiStyles: ReadonlyArray<readonly [UiStyle, string]> = [
  ["material", "Material"], ["ios", "iOS"],
];
export const defaultMaterialColor = "#007aff";
export const materialColorPresets = [
  [defaultMaterialColor, "蓝色"], ["#6750a4", "紫色"], ["#386a20", "绿色"],
  ["#006a6a", "青色"], ["#a04100", "橙色"], ["#984061", "玫瑰"],
] as const;
export const resolvedMaterialColor = computed(() => /^#[0-9a-f]{6}$/i.test(settings.value.materialColor ?? "")
  ? settings.value.materialColor!.toLowerCase() : defaultMaterialColor);
export const readingThemes: ReadonlyArray<readonly [ReadingTheme, string]> = [
  ["white", "纯白"], ["paper", "纸张"], ["sepia", "护眼"], ["dark", "深色"],
];

const systemDark = ref(false);
export const resolvedAppTheme = computed<"light" | "dark">(() =>
  settings.value.theme === "system" ? systemDark.value ? "dark" : "light" : settings.value.theme);
export const resolvedReadingTheme = computed(() => settings.value.reader.themes[resolvedAppTheme.value]);

export function setReaderTheme(theme: ReadingTheme): void {
  if (settings.value.theme === "system") return;
  settings.value.reader.themes[settings.value.theme] = theme;
}

/** System appearance is display state; it never writes back to settings.json. */
export function initializeTheme(): () => void {
  const media = window.matchMedia("(prefers-color-scheme: dark)");
  systemDark.value = media.matches;
  const onChange = (event: MediaQueryListEvent) => { systemDark.value = event.matches; };
  media.addEventListener("change", onChange);
  const stop = watch([resolvedAppTheme, resolvedReadingTheme, () => settings.value.uiStyle], ([app, reader, style]) => {
    document.documentElement.dataset.theme = app;
    document.documentElement.dataset.readerTheme = reader;
    document.documentElement.dataset.uiStyle = style;
    document.documentElement.classList.toggle("dark", app === "dark");
    document.documentElement.classList.toggle("k-ios", style === "ios");
    document.documentElement.classList.toggle("k-material", style === "material");
  }, { immediate: true, flush: "sync" });
  const stopColor = watch([resolvedMaterialColor, () => settings.value.uiStyle], ([color, style]) => {
    for (const [token, value] of Object.entries(materialColors(color))) {
      const property = `--k-color-${token}`;
      if (style === "material") document.documentElement.style.setProperty(property, value);
      else document.documentElement.style.removeProperty(property);
    }
  }, { immediate: true, flush: "sync" });
  return () => { stop(); stopColor(); media.removeEventListener("change", onChange); };
}

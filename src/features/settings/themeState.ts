import { computed, ref, watch } from "vue";
import type { AppTheme, ReadingTheme } from "../../api/types";
import { settings } from "./settingsStore";

export const appThemes: ReadonlyArray<readonly [AppTheme, string]> = [
  ["light", "白天"], ["dark", "夜间"], ["system", "跟随系统"],
];
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
  const stop = watch([resolvedAppTheme, resolvedReadingTheme], ([app, reader]) => {
    document.documentElement.dataset.theme = app;
    document.documentElement.dataset.readerTheme = reader;
  }, { immediate: true, flush: "sync" });
  return () => { stop(); media.removeEventListener("change", onChange); };
}

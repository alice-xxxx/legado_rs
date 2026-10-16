import type { ReaderSettings } from "../../api/types";

export type ReadingMode = ReaderSettings["readingMode"];
export type AnimatedReadingMode = Exclude<ReadingMode, "none" | "vertical">;

export const readingModes = [
  { value: "none", label: "无动画" },
  { value: "cover", label: "覆盖" },
  { value: "slide", label: "左右平移" },
  { value: "slide-vertical", label: "上下翻页" },
  { value: "fade", label: "淡入淡出" },
  { value: "flip", label: "3D 翻转" },
  { value: "simulation", label: "仿真卷页" },
  { value: "vertical", label: "上下连续滚动" },
] as const satisfies readonly { value: ReadingMode; label: string }[];

export function isReadingMode(value: string): value is ReadingMode {
  return readingModes.some(mode => mode.value === value);
}

export function hasPageTurnAnimation(mode: ReadingMode): mode is AnimatedReadingMode {
  return mode !== "none" && mode !== "vertical";
}

export function isVerticalReadingMode(mode: ReadingMode): boolean {
  return mode === "vertical" || mode === "slide-vertical";
}

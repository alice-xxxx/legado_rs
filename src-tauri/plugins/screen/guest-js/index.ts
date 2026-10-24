import { addPluginListener, invoke, isTauri } from "@tauri-apps/api/core";

export type Orientation = "portrait" | "landscape" | "portrait-primary" | "portrait-secondary"
  | "landscape-primary" | "landscape-secondary" | "auto" | "restore";
/** Light/dark describes the system bar foreground, not its background. */
export type SystemUiStyle = "auto" | "light" | "dark";
export interface ScreenOptions {
  fullscreen?: boolean;
  systemUiHidden?: boolean;
  systemUiStyle?: SystemUiStyle;
  orientation?: Orientation;
  keepAwake?: boolean;
  /** 0..1; null restores the brightness from before this plugin changed it. */
  brightness?: number | null;
}
export type ScreenControl = keyof ScreenOptions;
export interface ScreenUpdate { applied: ScreenControl[]; unsupported: ScreenControl[]; }
export interface ScreenInfo {
  width: number;
  height: number;
  scale: number;
  fullscreen: boolean;
  systemUiHidden: boolean | null;
  systemUiStyle: SystemUiStyle | null;
  orientation: "portrait" | "landscape";
  keepAwake: boolean;
  brightness: number | null;
  safeArea: { top: number; right: number; bottom: number; left: number };
  capabilities: Record<ScreenControl, boolean>;
}
export interface ScreenError { operation: ScreenControl; message: string; }

export async function apply(options: ScreenOptions): Promise<ScreenUpdate> {
  const level = options.brightness;
  if (level != null && (!Number.isFinite(level) || level < 0 || level > 1)) {
    throw new RangeError("Brightness must be between 0 and 1, or null to restore it");
  }
  if (!isTauri()) return { applied: [], unsupported: Object.keys(options) as ScreenControl[] };
  return invoke("plugin:screen|apply", { options: { ...options, brightness: level === null ? -1 : level } });
}

async function set<K extends ScreenControl>(control: K, value: ScreenOptions[K]): Promise<boolean> {
  return (await apply({ [control]: value })).applied.includes(control);
}
export const setFullscreen = (enabled: boolean) => set("fullscreen", enabled);
export const setSystemUiHidden = (hidden: boolean) => set("systemUiHidden", hidden);
export const setSystemUiStyle = (style: SystemUiStyle) => set("systemUiStyle", style);
export const setOrientation = (orientation: Orientation) => set("orientation", orientation);
export const setKeepAwake = (enabled: boolean) => set("keepAwake", enabled);
export const setBrightness = (level: number | null) => set("brightness", level);
export const getInfo = () => invoke<ScreenInfo>("plugin:screen|get_info");
export const reset = () => invoke<void>("plugin:screen|reset");

/** iOS geometry requests can fail asynchronously after being submitted. */
export async function onError(handler: (error: ScreenError) => void): Promise<() => Promise<void>> {
  if (!isTauri() || !/Android|iPhone|iPad|iPod/.test(navigator.userAgent)
    && !(navigator.maxTouchPoints > 1 && /Macintosh/.test(navigator.userAgent))) return async () => {};
  const listener = await addPluginListener("screen", "error", handler);
  return () => listener.unregister();
}

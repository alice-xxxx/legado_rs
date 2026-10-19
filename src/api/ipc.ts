import { invoke, isTauri } from "@tauri-apps/api/core";

export async function command<T>(name: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) {
    throw new Error("此功能需要在应用中使用。");
  }
  return invoke<T>(name, args);
}

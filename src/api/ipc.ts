import { invoke, isTauri } from "@tauri-apps/api/core";

function requireTauri(): void {
  if (!isTauri()) {
    throw new Error("此功能需要在应用中使用。");
  }
}

export async function command<T>(name: string, args?: Record<string, unknown>): Promise<T> {
  requireTauri();
  return invoke<T>(name, args);
}

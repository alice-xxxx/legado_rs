import { ref } from "vue";

export type NoticeKind = "success" | "error";

export const toast = ref("");
export const toastKind = ref<NoticeKind>("success");
let toastTimer: ReturnType<typeof setTimeout> | undefined;

export function notify(message: string, kind: NoticeKind = "success"): void {
  toast.value = message;
  toastKind.value = kind;
  if (toastTimer) clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (toast.value = ""), 3200);
}

export function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function dismissToast(): void {
  toast.value = "";
}

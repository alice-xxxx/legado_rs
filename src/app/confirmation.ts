import { shallowRef } from "vue";

interface Confirmation {
  title: string;
  message: string;
  confirmLabel: string;
  resolve: (confirmed: boolean) => void;
}

export const confirmation = shallowRef<Confirmation | null>(null);

export function answerConfirmation(confirmed: boolean): void {
  const pending = confirmation.value;
  confirmation.value = null;
  pending?.resolve(confirmed);
}

export function confirmAction(title: string, message: string, confirmLabel: string): Promise<boolean> {
  answerConfirmation(false);
  return new Promise(resolve => { confirmation.value = { title, message, confirmLabel, resolve }; });
}

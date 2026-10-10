import { ref } from "vue";

/** Shared read-only gates for screens that cannot safely mutate during recovery. */
export const shelfBatchRecoveryRequired = ref(false);
export const shelfBatchRecoveryTitle = ref("需要重启应用完成恢复");
export const shelfBatchRecoveryMessage = ref("");

let frontendRestoreRevision = 0;

export function getFrontendRestoreRevision(): number {
  return frontendRestoreRevision;
}

export function beginFrontendRestoreTransition(): number {
  frontendRestoreRevision += 1;
  return frontendRestoreRevision;
}

export function lockShelfBatchRecovery(title: string, message: string): void {
  shelfBatchRecoveryRequired.value = true;
  shelfBatchRecoveryTitle.value = title;
  shelfBatchRecoveryMessage.value = message;
}

export function clearShelfBatchRecovery(): void {
  shelfBatchRecoveryRequired.value = false;
  shelfBatchRecoveryTitle.value = "需要重启应用完成恢复";
  shelfBatchRecoveryMessage.value = "";
}

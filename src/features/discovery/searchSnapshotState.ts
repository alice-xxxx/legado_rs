import { computed, shallowRef } from "vue";
import type { ResourceDescriptor } from "../../api/types";

const pendingSearchSnapshot = shallowRef<ResourceDescriptor | null>(null);

/** Last immutable search result descriptor, retained while the search screen is unmounted. */
export const searchSnapshot = computed(() => pendingSearchSnapshot.value);

export function publishSearchSnapshot(descriptor: ResourceDescriptor | null): void {
  if (pendingSearchSnapshot.value?.resourceId === descriptor?.resourceId) return;
  pendingSearchSnapshot.value = descriptor;
}

/** Clear only the descriptor that was consumed, preserving a newer publication. */
export function clearSearchSnapshot(resourceId?: string): void {
  if (resourceId && pendingSearchSnapshot.value?.resourceId !== resourceId) return;
  pendingSearchSnapshot.value = null;
}

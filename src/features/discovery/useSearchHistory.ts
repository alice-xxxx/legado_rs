import { computed } from "vue";
import { history, removeSearchHistory } from "../settings/historyStore";

export function useSearchHistory() {
  const searchHistoryEntries = computed(() => history.value.search.entries);

  function removeSearchHistoryEntry(query: string): void {
    void removeSearchHistory(query).catch(console.error);
  }

  function clearSearchHistoryEntries(): void {
    void removeSearchHistory().catch(console.error);
  }

  return { searchHistoryEntries, removeSearchHistoryEntry, clearSearchHistoryEntries };
}

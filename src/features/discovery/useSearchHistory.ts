import { computed } from "vue";
import { history } from "../settings/historyStore";

export function useSearchHistory() {
  const searchHistoryEntries = computed(() => history.value.search.entries);

  function removeSearchHistoryEntry(query: string): void {
    history.value.search.entries = history.value.search.entries.filter((entry) => entry.query !== query);
  }

  function clearSearchHistoryEntries(): void {
    history.value.search.entries = [];
  }

  return { searchHistoryEntries, removeSearchHistoryEntry, clearSearchHistoryEntries };
}

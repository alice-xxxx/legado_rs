import { ref } from "vue";
import { clearSearchHistory, deleteSearchHistory, getSearchHistory } from "../../api/search";
import { readResource } from "../../api/resources";
import { type ResourceDescriptor, type SearchHistoryEntry, type SearchHistoryResource } from "../../api/types";

/** 搜索历史状态与回滚刷新放在发现功能内，避免根组件管理记录细节。 */
export function useSearchHistory(errorText: (error: unknown) => string, getRestoreRevision: () => number) {
  const searchHistoryEntries = ref<SearchHistoryEntry[]>([]);
  const searchHistoryBusy = ref(false);
  const searchHistoryError = ref("");
  let historyReadRevision = 0;

  async function refreshSearchHistory(descriptor?: ResourceDescriptor): Promise<void> {
    const restoreRevision = getRestoreRevision();
    const readRevision = ++historyReadRevision;
    const resource = descriptor ?? await getSearchHistory();
    const document = await readResource<SearchHistoryResource>(resource);
    if (restoreRevision !== getRestoreRevision() || readRevision !== historyReadRevision) return;
    searchHistoryEntries.value = Array.isArray(document.entries) ? document.entries : [];
  }

  async function reloadSearchHistoryAfterError(): Promise<void> {
    try {
      await refreshSearchHistory(await getSearchHistory());
    } catch {
      // 命令与恢复刷新都失败时，保留上一次显示的记录。
    }
  }

  async function mutateSearchHistory(label: string, mutation: () => Promise<ResourceDescriptor>): Promise<void> {
    if (searchHistoryBusy.value) return;
    searchHistoryBusy.value = true;
    const restoreRevision = getRestoreRevision();
    searchHistoryError.value = "";
    let descriptor: ResourceDescriptor;
    try {
      descriptor = await mutation();
    } catch (error) {
      if (restoreRevision === getRestoreRevision()) {
        searchHistoryError.value = `${label}失败：${errorText(error)}`;
        await reloadSearchHistoryAfterError();
      }
      searchHistoryBusy.value = false;
      return;
    }
    if (restoreRevision !== getRestoreRevision()) {
      searchHistoryBusy.value = false;
      return;
    }
    try {
      await refreshSearchHistory(descriptor);
    } catch (error) {
      if (restoreRevision === getRestoreRevision()) {
        searchHistoryError.value = `${label}已提交，但刷新记录失败：${errorText(error)}`;
        await reloadSearchHistoryAfterError();
      }
    } finally {
      searchHistoryBusy.value = false;
    }
  }

  function removeSearchHistoryEntry(query: string): Promise<void> {
    return mutateSearchHistory("删除搜索记录", () => deleteSearchHistory(query));
  }

  function clearSearchHistoryEntries(): Promise<void> {
    return mutateSearchHistory("清空搜索记录", clearSearchHistory);
  }

  return {
    searchHistoryEntries,
    searchHistoryBusy,
    searchHistoryError,
    refreshSearchHistory,
    removeSearchHistoryEntry,
    clearSearchHistoryEntries,
  };
}

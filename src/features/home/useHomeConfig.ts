import { ref, type ComputedRef } from "vue";
import { getHomeConfig, listDiscoveryCategories, saveHomeConfig } from "../../api/discovery";
import { readResource } from "../../api/resources";
import type {
  DiscoveryCategoriesResource,
  DiscoveryCategory,
  HomeConfigDocument,
  ResourceDescriptor,
  SourceMetadata,
} from "../../api/types";

interface HomeConfigOptions {
  enabledSources: ComputedRef<SourceMetadata[]>;
  getRestoreRevision: () => number;
}

function emptyHomeConfig(): HomeConfigDocument {
  return {
    schemaVersion: 1,
    tabs: [{ id: "tab-home", title: "\u4E3B\u9875", sortOrder: 0, sections: [] }],
  };
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** Configuration and category state for the home configuration editor screen. */
export function useHomeConfig(options: HomeConfigOptions) {
  const homeConfig = ref<HomeConfigDocument>(emptyHomeConfig());
  const homeConfigLoading = ref(true);
  const homeConfigLoaded = ref(false);
  const homeConfigSaving = ref(false);
  const homeConfigError = ref("");
  const homeEditorCategories = ref<Record<string, DiscoveryCategory[]>>({});
  const homeEditorLoadingSourceIds = ref<string[]>([]);
  const homeEditorCategoryErrors = ref<Record<string, string>>({});
  const homeEditorCategoryLoads = new Map<string, Promise<void>>();
  let configRequestRevision = 0;
  let editorRequestRevision = 0;

  function isEnabledSource(sourceId: string): boolean {
    return options.enabledSources.value.some(
      (source) => source.id === sourceId && source.enabled && source.isRss !== true,
    );
  }

  async function refreshHomeConfig(descriptor?: ResourceDescriptor): Promise<boolean> {
    const restoreRevision = options.getRestoreRevision();
    const requestRevision = ++configRequestRevision;
    const isCurrent = () => restoreRevision === options.getRestoreRevision()
      && requestRevision === configRequestRevision;
    homeConfigLoading.value = true;
    homeConfigError.value = "";
    try {
      const resource = descriptor ?? await getHomeConfig();
      const document = await readResource<HomeConfigDocument>(resource);
      if (!isCurrent()) return false;
      const tabs = Array.isArray(document.tabs)
        ? document.tabs.map((tab) => ({ ...tab, sections: Array.isArray(tab.sections) ? tab.sections : [] }))
        : [];
      homeConfig.value = { schemaVersion: document.schemaVersion ?? 1, tabs };
      homeConfigLoaded.value = true;
      return true;
    } catch (error) {
      if (isCurrent()) {
        homeConfigLoaded.value = false;
        homeConfigError.value = errorText(error);
      }
      return false;
    } finally {
      if (isCurrent()) homeConfigLoading.value = false;
    }
  }

  async function loadHomeEditorCategories(sourceId: string): Promise<void> {
    if (!isEnabledSource(sourceId)) return;
    const restoreRevision = options.getRestoreRevision();
    const editorRevision = editorRequestRevision;
    const isCurrent = () => restoreRevision === options.getRestoreRevision()
      && editorRevision === editorRequestRevision;
    const current = homeEditorCategoryLoads.get(sourceId);
    if (current) return current;
    homeEditorLoadingSourceIds.value = [...new Set([...homeEditorLoadingSourceIds.value, sourceId])];
    const errors = { ...homeEditorCategoryErrors.value };
    delete errors[sourceId];
    homeEditorCategoryErrors.value = errors;

    const request = (async () => {
      try {
        const response = await listDiscoveryCategories(sourceId);
        const document = await readResource<DiscoveryCategoriesResource>(response.resource);
        if (!isCurrent() || !isEnabledSource(sourceId)) return;
        homeEditorCategories.value = {
          ...homeEditorCategories.value,
          [sourceId]: Array.isArray(document.categories) ? document.categories : [],
        };
      } catch (error) {
        if (isCurrent()) {
          homeEditorCategoryErrors.value = { ...homeEditorCategoryErrors.value, [sourceId]: errorText(error) };
        }
      } finally {
        if (isCurrent()) {
          homeEditorLoadingSourceIds.value = homeEditorLoadingSourceIds.value.filter((id) => id !== sourceId);
        }
      }
    })();
    homeEditorCategoryLoads.set(sourceId, request);
    try {
      await request;
    } finally {
      if (homeEditorCategoryLoads.get(sourceId) === request) homeEditorCategoryLoads.delete(sourceId);
    }
  }

  async function saveHomeConfigDraft(config: HomeConfigDocument): Promise<boolean> {
    if (homeConfigSaving.value) return false;
    const restoreRevision = options.getRestoreRevision();
    const isCurrent = () => restoreRevision === options.getRestoreRevision();
    homeConfigSaving.value = true;
    homeConfigError.value = "";
    try {
      let descriptor: ResourceDescriptor;
      try {
        descriptor = await saveHomeConfig(config);
      } catch (error) {
        if (isCurrent()) homeConfigError.value = errorText(error);
        return false;
      }
      if (!isCurrent()) return false;
      if (await refreshHomeConfig(descriptor)) return isCurrent();
      if (!isCurrent()) return false;
      return await refreshHomeConfig();
    } finally {
      if (isCurrent()) homeConfigSaving.value = false;
    }
  }

  function resetForRestore(): void {
    configRequestRevision += 1;
    editorRequestRevision += 1;
    homeConfig.value = emptyHomeConfig();
    homeConfigLoading.value = true;
    homeConfigLoaded.value = false;
    homeConfigSaving.value = false;
    homeConfigError.value = "";
    homeEditorCategories.value = {};
    homeEditorLoadingSourceIds.value = [];
    homeEditorCategoryErrors.value = {};
    homeEditorCategoryLoads.clear();
  }

  return {
    homeConfig,
    homeConfigLoading,
    homeConfigLoaded,
    homeConfigSaving,
    homeConfigError,
    homeEditorCategories,
    homeEditorLoadingSourceIds,
    homeEditorCategoryErrors,
    refreshHomeConfig,
    loadHomeEditorCategories,
    saveHomeConfigDraft,
    resetForRestore,
  };
}

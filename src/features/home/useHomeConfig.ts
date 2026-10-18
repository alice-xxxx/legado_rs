import { computed, ref, type ComputedRef } from "vue";
import { listDiscoveryCategories } from "../../api/discovery";
import { readCategories } from "../../api/database";
import { settings as sharedSettings } from "../settings/settingsStore";
import type {
  DiscoveryCategory,
  SourceMetadata,
} from "../../api/types";

interface HomeConfigOptions {
  enabledSources: ComputedRef<SourceMetadata[]>;
  getRestoreRevision: () => number;
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** Configuration and category state for the home configuration editor screen. */
export function useHomeConfig(options: HomeConfigOptions) {
  const homeConfig = computed(() => sharedSettings.value.home);
  const homeEditorCategories = ref<Record<string, DiscoveryCategory[]>>({});
  const homeEditorLoadingSourceIds = ref<string[]>([]);
  const homeEditorCategoryErrors = ref<Record<string, string>>({});
  const homeEditorCategoryLoads = new Map<string, Promise<void>>();
  let editorRequestRevision = 0;

  function isEnabledSource(sourceId: string): boolean {
    return options.enabledSources.value.some(
      (source) => source.id === sourceId && source.enabled && source.isRss !== true,
    );
  }

  async function loadHomeEditorCategories(sourceId: string): Promise<void> {
    if (!isEnabledSource(sourceId)) return;
    if (homeEditorCategories.value[sourceId]) return;
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
        await listDiscoveryCategories(sourceId);
        const document = await readCategories(sourceId);
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

  function resetForRestore(): void {
    editorRequestRevision += 1;
    homeEditorCategories.value = {};
    homeEditorLoadingSourceIds.value = [];
    homeEditorCategoryErrors.value = {};
    homeEditorCategoryLoads.clear();
  }

  return {
    homeConfig,
    homeEditorCategories,
    homeEditorLoadingSourceIds,
    homeEditorCategoryErrors,
    loadHomeEditorCategories,
    resetForRestore,
  };
}

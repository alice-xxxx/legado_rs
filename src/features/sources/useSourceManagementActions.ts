import { ref, type Ref } from "vue";
import { sources, bookSourceCandidates, mutateSources as mutateSource, readSourceDefinitionJson, saveSourceMetadata, saveSourceDefinition, exportSourceDefinitions, type SourceMetadataChanges } from "./sourcesStore";
import type { SourceMetadata } from "../../api/types";
import { shelfBatchRecoveryRequired as recoveryRequired } from "../../app/recoveryState";

interface SourceManagementOptions {
  selectedSourceIds: Ref<string[]>;
  selectedSourceRows: Ref<string[]>;
  sourceBatchBusy: Ref<boolean>;
  sourceExportBusy: Ref<boolean>;
  sourceImportOpen: Ref<boolean>;
  sourceImportBusy: Ref<boolean>;
  sourceImportUrl: Ref<string>;
  pendingSourceRemoval: Ref<string[]>;
  editingSource: Ref<SourceMetadata | null>;
  getRestoreFrontendRevision: () => number;
  errorText: (error: unknown) => string;
  notify: (message: string, kind?: "success" | "error") => void;
}

/** 处理书源列表、批量维护、导入导出、移除和元数据编辑。 */
export function useSourceManagementActions(options: SourceManagementOptions) {
  const {
    selectedSourceIds, selectedSourceRows,
    sourceBatchBusy, sourceExportBusy, editingSource,
    sourceImportOpen, sourceImportBusy, sourceImportUrl, pendingSourceRemoval,
    getRestoreFrontendRevision, errorText, notify,
  } = options;
  const sourceEditBusy = ref(false);
  const sourceEditJson = ref("");
  const sourceEditOriginalJson = ref("");
  const sourceRemovalBusy = ref(false);

  async function changeSourceEnabled(source: SourceMetadata, enabled: boolean): Promise<void> {
    if (recoveryRequired.value || sourceBatchBusy.value || sourceExportBusy.value || sourceRemovalBusy.value || sourceEditBusy.value) return;
    const revision = getRestoreFrontendRevision();
    try {
      await saveSourceMetadata(source.id, { enabled });
      if (revision !== getRestoreFrontendRevision()) return;
      if (!enabled || source.isRss === true) selectedSourceIds.value = selectedSourceIds.value.filter((id) => id !== source.id);
      else if (!selectedSourceIds.value.includes(source.id)) selectedSourceIds.value.push(source.id);
      notify(`${source.name} 已${enabled ? "启用" : "停用"}。`);
    } catch (error) {
      if (revision !== getRestoreFrontendRevision()) return;
      notify(`更新书源状态失败：${errorText(error)}`, "error");
    }
  }

  async function patchSelectedSources(
    patch: SourceMetadataChanges,
    operation: string,
    noChangeMessage: string,
  ): Promise<void> {
    if (recoveryRequired.value || sourceBatchBusy.value || sourceExportBusy.value || sourceRemovalBusy.value
      || sourceEditBusy.value || !selectedSourceRows.value.length) return;
    const normalizedPatch = { ...patch };
    if (normalizedPatch.group !== undefined) normalizedPatch.group = normalizedPatch.group?.trim() || null;
    const matchesPatch = (source: SourceMetadata) =>
      (normalizedPatch.enabled === undefined || source.enabled === normalizedPatch.enabled) &&
      (normalizedPatch.group === undefined || (source.group ?? "") === (normalizedPatch.group ?? ""));
    const targets = selectedSourceRows.value
      .map((id) => sources.value.find((source) => source.id === id))
      .filter((source): source is SourceMetadata => source !== undefined && !matchesPatch(source));
    if (!targets.length) {
      notify(noChangeMessage);
      return;
    }

    const previousSearchSelection = [...selectedSourceIds.value];
    const restoreRevision = getRestoreFrontendRevision();
    sourceBatchBusy.value = true;
    let completed = 0;
    let failure: unknown = null;
    try {
      for (const source of targets) {
        await saveSourceMetadata(source.id, normalizedPatch);
        if (restoreRevision !== getRestoreFrontendRevision()) break;
        completed += 1;
      }
    } catch (error) {
      failure = error;
    }

    if (restoreRevision !== getRestoreFrontendRevision()) {
      sourceBatchBusy.value = false;
      return;
    }
    const searchSelection = new Set(previousSearchSelection);
    if (normalizedPatch.enabled !== undefined) {
      for (const target of targets) {
        const latest = sources.value.find((source) => source.id === target.id);
        if (latest?.enabled && latest.isRss !== true) searchSelection.add(target.id);
        else searchSelection.delete(target.id);
      }
    }
    selectedSourceIds.value = [...searchSelection];

    sourceBatchBusy.value = false;
    if (restoreRevision !== getRestoreFrontendRevision()) return;
    if (failure) {
      notify(`批量操作中断，已确认完成 ${completed}/${targets.length} 个书源：${errorText(failure)}`, "error");
      return;
    }
    notify(`已${operation} ${completed} 个所选书源。`);
  }

  async function setSelectedSourcesEnabled(enabled: boolean): Promise<void> {
    await patchSelectedSources(
      { enabled },
      enabled ? "启用" : "停用",
      `所选书源已全部${enabled ? "启用" : "停用"}。`,
    );
  }

  async function setSelectedSourcesGroup(group: string | null): Promise<void> {
    const normalizedGroup = group?.trim() || null;
    await patchSelectedSources(
      { group: normalizedGroup },
      normalizedGroup ? `分组为“${normalizedGroup}”` : "清除分组",
      normalizedGroup ? `所选书源已全部属于“${normalizedGroup}”分组。` : "所选书源已全部未分组。",
    );
  }

  async function beginSourceEdit(source: SourceMetadata): Promise<void> {
    if (recoveryRequired.value || sourceEditBusy.value || sourceRemovalBusy.value) return;
    const revision = getRestoreFrontendRevision();
    sourceEditBusy.value = true;
    try {
      const json = await readSourceDefinitionJson(source.id);
      if (revision !== getRestoreFrontendRevision() || !sources.value.some(s => s.id === source.id)) return;
      sourceEditJson.value = json;
      sourceEditOriginalJson.value = json;
      editingSource.value = sources.value.find(s => s.id === source.id) ?? source;
    } catch (error) {
      if (revision === getRestoreFrontendRevision()) notify(`读取完整书源定义失败：${errorText(error)}`, "error");
    } finally {
      sourceEditBusy.value = false;
    }
  }

  function closeSourceEdit(): void {
    if (sourceEditBusy.value) return;
    editingSource.value = null;
    sourceEditJson.value = "";
    sourceEditOriginalJson.value = "";
  }

  async function saveSourceDefinitionJson(): Promise<void> {
    const source = editingSource.value;
    if (!source || recoveryRequired.value || sourceEditBusy.value || sourceRemovalBusy.value
      || sourceBatchBusy.value || sourceImportBusy.value) return;
    const definitionJson = sourceEditJson.value;
    try {
      const value: unknown = JSON.parse(definitionJson);
      if (!value || typeof value !== "object" || Array.isArray(value)) {
        notify("书源 JSON 必须是一个对象。", "error"); return;
      }
    } catch (error) {
      notify(`JSON 语法错误：${errorText(error)}`, "error"); return;
    }
    const revision = getRestoreFrontendRevision();
    sourceEditBusy.value = true;
    try {
      await saveSourceDefinition(source.id, definitionJson);
      if (revision !== getRestoreFrontendRevision()) return;
      editingSource.value = null;
      sourceEditJson.value = "";
      sourceEditOriginalJson.value = "";
      notify("书源配置已保存，新规则会用于后续搜索和解析。");
    } catch (error) {
      if (revision === getRestoreFrontendRevision()) notify(`保存书源失败：${errorText(error)}`, "error");
    } finally {
      sourceEditBusy.value = false;
    }
  }

  async function createSourceFromJson(sourceJson: string): Promise<boolean> {
    if (recoveryRequired.value || sourceImportBusy.value || sourceBatchBusy.value
      || sourceExportBusy.value || sourceEditBusy.value || sourceRemovalBusy.value) return false;
    // Use the same full-schema Legado importer as file and URL imports.
    // Rust owns source IDs, validation, and persisted definitions.
    let definition: unknown;
    try {
      definition = JSON.parse(sourceJson);
      if (!definition || typeof definition !== "object" || Array.isArray(definition))
        throw new Error("手动新建来源必须是一个完整 JSON 对象");
      const value = definition as Record<string, unknown>;
      const name = value.bookSourceName ?? value.name ?? value.sourceName;
      const url = value.bookSourceUrl ?? value.url ?? value.sourceUrl;
      if (typeof name !== "string" || !name.trim()) throw new Error("请填写 bookSourceName");
      if (typeof url !== "string" || !url.trim()) throw new Error("请填写 bookSourceUrl");
    } catch (error) {
      notify(`新建来源失败：${errorText(error)}`, "error");
      return false;
    }
    const restoreRevision = getRestoreFrontendRevision();
    sourceImportBusy.value = true;
    try {
      await mutateSource({
        kind: "save",
        id: null,
        json: sourceJson,
      });
      if (restoreRevision !== getRestoreFrontendRevision()) return false;
      sourceImportOpen.value = false;
      notify("来源配置已保存。");
      return true;
    } catch (error) {
      if (restoreRevision === getRestoreFrontendRevision()) notify(`新建来源失败：${errorText(error)}`, "error");
      return false;
    } finally {
      sourceImportBusy.value = false;
    }
  }

  async function importSourceFileFromPicker(): Promise<void> {
    if (recoveryRequired.value || sourceImportBusy.value || sourceBatchBusy.value || sourceExportBusy.value
      || sourceEditBusy.value || sourceRemovalBusy.value) return;
    const revision = getRestoreFrontendRevision();
    sourceImportBusy.value = true;
    try {
      const response = await mutateSource({ kind: "local" });
      if ("cancelled" in response && response.cancelled) return;
      if (revision !== getRestoreFrontendRevision()) return;
      selectedSourceIds.value = bookSourceCandidates.value.map((source) => source.id);
      sourceImportOpen.value = false;
      notify("书源已导入。");
    } catch (error) {
      if (revision !== getRestoreFrontendRevision()) return;
      notify(`书源导入失败：${errorText(error)}`, "error");
    } finally {
      sourceImportBusy.value = false;
    }
  }

  async function importSourceFromUrl(): Promise<void> {
    const url = sourceImportUrl.value.trim();
    if (!url || recoveryRequired.value || sourceImportBusy.value || sourceBatchBusy.value || sourceExportBusy.value
      || sourceEditBusy.value || sourceRemovalBusy.value) return;
    const revision = getRestoreFrontendRevision();
    sourceImportBusy.value = true;
    try {
      await mutateSource({
        kind: "url",
        url,
      });
      if (revision !== getRestoreFrontendRevision()) return;
      selectedSourceIds.value = bookSourceCandidates.value.map((source) => source.id);
      sourceImportUrl.value = "";
      sourceImportOpen.value = false;
      notify("书源已从链接导入。");
    } catch (error) {
      if (revision !== getRestoreFrontendRevision()) return;
      notify(`链接导入书源失败：${errorText(error)}`, "error");
    } finally {
      sourceImportBusy.value = false;
    }
  }

  async function exportSelectedSources(): Promise<void> {
    const sourceIds = [...selectedSourceRows.value].filter((id) => sources.value.some((source) => source.id === id));
    if (!sourceIds.length || sourceExportBusy.value || sourceBatchBusy.value || sourceImportBusy.value
      || sourceEditBusy.value || sourceRemovalBusy.value) return;
    sourceExportBusy.value = true;
    try {
      const json = await exportSourceDefinitions(sourceIds);
      const url = URL.createObjectURL(new Blob([json], { type: "application/json;charset=utf-8" }));
      const anchor = window.document.createElement("a");
      anchor.href = url;
      anchor.download = "book-sources.json";
      window.document.body.append(anchor);
      try {
        anchor.click();
        notify(`已导出 ${sourceIds.length} 个书源。`);
      } finally {
        anchor.remove();
        window.setTimeout(() => URL.revokeObjectURL(url), 1000);
      }
    } catch (error) {
      notify(`导出书源失败：${errorText(error)}`, "error");
    } finally {
      sourceExportBusy.value = false;
    }
  }

  async function deleteSelectedSources(): Promise<void> {
    if (!pendingSourceRemoval.value.length || recoveryRequired.value || sourceExportBusy.value || sourceBatchBusy.value
      || sourceImportBusy.value || sourceEditBusy.value || sourceRemovalBusy.value) return;
    const targets = [...pendingSourceRemoval.value];
    const revision = getRestoreFrontendRevision();
    sourceRemovalBusy.value = true;
    try {
      const result = await mutateSource({
        kind: "delete",
        sourceIds: targets,
      });
      if (revision !== getRestoreFrontendRevision()) return;
      // Rust has committed removal. Do not leave a confirm dialog that can
      // accidentally submit the same destructive operation a second time.
      const removed = new Set(targets);
      selectedSourceRows.value = selectedSourceRows.value.filter((id) => !removed.has(id));
      pendingSourceRemoval.value = [];
      if (revision !== getRestoreFrontendRevision()) return;
      const cleanupWarning = "cleanupWarning" in result ? result.cleanupWarning : undefined;
      const cleanupPending = "cleanupPending" in result && result.cleanupPending;
      notify(cleanupWarning || "所选书源已移除。", cleanupPending ? "error" : "success");
    } catch (error) {
      if (revision !== getRestoreFrontendRevision()) return;
      notify(`删除书源失败：${errorText(error)}`, "error");
    } finally {
      sourceRemovalBusy.value = false;
    }
  }

  return {
    sourceEditBusy,
    sourceRemovalBusy,
    changeSourceEnabled,
    patchSelectedSources,
    setSelectedSourcesEnabled,
    setSelectedSourcesGroup,
    beginSourceEdit,
    closeSourceEdit,
    saveSourceDefinitionJson,
    sourceEditJson,
    sourceEditOriginalJson,
    importSourceFileFromPicker,
    importSourceFromUrl,
    createSourceFromJson,
    exportSelectedSources,
    deleteSelectedSources,
  };
}

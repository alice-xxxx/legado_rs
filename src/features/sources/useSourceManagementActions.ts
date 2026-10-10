import { ref, type Ref } from "vue";
import { mutateSource } from "../../api/sources";
import { readResource } from "../../api/resources";
import { type ResourceDescriptor, type SourceDefinitionsResource, type SourceMetadata, type SourceMutationResponse } from "../../api/types";
import { removeSourceJsonValue, setSourceJsonValue } from "./sourceJsonEdits";

type SourceMetadataChanges = { enabled?: boolean; group?: string | null };

function applySourceMetadataChanges(
  definitionJson: string,
  changes: SourceMetadataChanges,
): string {
  const parsed: unknown = JSON.parse(definitionJson);
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("书源 JSON 必须是一个对象");
  }
  const definition = parsed as Record<string, unknown>;
  let result = definitionJson;

  const updateAliases = (keys: string[], value: string | boolean | null): void => {
    const present = keys.filter(key => Object.prototype.hasOwnProperty.call(definition, key));
    const targets = present.length ? present : value === null ? [] : [keys[0]!];
    for (const key of targets) {
      result = value === null
        ? removeSourceJsonValue(result, [key])
        : setSourceJsonValue(result, [key], JSON.stringify(value));
    }
  };

  if (changes.group !== undefined) updateAliases(["bookSourceGroup", "group"], changes.group?.trim() || null);
  if (changes.enabled !== undefined) updateAliases(["enabled", "bookSourceEnabled"], changes.enabled);
  return result;
}

function sourceResource(result: SourceMutationResponse): ResourceDescriptor {
  if (!("resource" in result)) throw new Error("Source mutation returned no source resource");
  return result.resource;
}

interface SourceManagementOptions {
  sources: Ref<SourceMetadata[]>;
  selectedSourceIds: Ref<string[]>;
  selectedSourceRows: Ref<string[]>;
  bookSourceCandidates: { readonly value: SourceMetadata[] };
  sourceBatchBusy: Ref<boolean>;
  sourceExportBusy: Ref<boolean>;
  sourceImportOpen: Ref<boolean>;
  sourceImportBusy: Ref<boolean>;
  sourceImportUrl: Ref<string>;
  pendingSourceRemoval: Ref<string[]>;
  editingSource: Ref<SourceMetadata | null>;
  sourceDefinitionsResource: Ref<ResourceDescriptor | null>;
  recoveryRequired: Readonly<Ref<boolean>>;
  /** 与书架启动刷新共用版本号，阻止较旧的响应覆盖新快照。 */
  advanceSourceSnapshotRevision: () => number;
  getSourceSnapshotRevision: () => number;
  getRestoreFrontendRevision: () => number;
  applySourcesSnapshot: (nextSources: SourceMetadata[]) => void;
  errorText: (error: unknown) => string;
  notify: (message: string, kind?: "success" | "error") => void;
}

/** 处理书源列表、批量维护、导入导出、移除和元数据编辑。 */
export function useSourceManagementActions(options: SourceManagementOptions) {
  const {
    sources, selectedSourceIds, selectedSourceRows, bookSourceCandidates,
    sourceBatchBusy, sourceExportBusy, editingSource,
    sourceDefinitionsResource,
    sourceImportOpen, sourceImportBusy, sourceImportUrl, pendingSourceRemoval,
    recoveryRequired,
    advanceSourceSnapshotRevision, getSourceSnapshotRevision, getRestoreFrontendRevision,
    applySourcesSnapshot, errorText, notify,
  } = options;
  const sourceEditBusy = ref(false);
  const sourceEditJson = ref("");
  const sourceEditOriginalJson = ref("");
  const sourceEditInitialUrl = ref("");
  const sourceRemovalBusy = ref(false);

  async function readSourceDefinitionJson(sourceId: string): Promise<string> {
    const descriptor = sourceDefinitionsResource.value;
    if (!descriptor) throw new Error("书源资源尚未加载");
    const document = await readResource<SourceDefinitionsResource>(descriptor);
    const source = document.sources.find((entry) => entry.id === sourceId);
    if (!source) throw new Error(`书源“${sourceId}”已不存在`);
    return source.definitionJson;
  }

  async function saveSourceMetadata(
    source: SourceMetadata,
    changes: SourceMetadataChanges,
  ): Promise<void> {
    const currentJson = await readSourceDefinitionJson(source.id);
    const json = applySourceMetadataChanges(currentJson, changes);
    const result = await mutateSource({ kind: "save", id: source.id, json });
    await refreshSources(sourceResource(result));
  }

  async function refreshSources(resource?: ResourceDescriptor): Promise<void> {
    const revision = advanceSourceSnapshotRevision();
    const restoreRevision = getRestoreFrontendRevision();
    if (resource) sourceDefinitionsResource.value = resource;
    const descriptor = sourceDefinitionsResource.value;
    if (!descriptor) throw new Error("Source resource has not been initialized");
    const response = await readResource<SourceDefinitionsResource>(descriptor);
    if (!Array.isArray(response.sources)) throw new Error("Source resource has an invalid format");
    if (revision !== getSourceSnapshotRevision() || restoreRevision !== getRestoreFrontendRevision()) return;
    // App.applySourcesSnapshot prunes disabled/deleted IDs. A normal refresh
    // must not silently reselect sources the user deliberately excluded.
    const nextSources = response.sources.map((entry): SourceMetadata => {
      const source = { ...entry };
      Reflect.deleteProperty(source, "definitionJson");
      return source;
    });
    applySourcesSnapshot(nextSources);
  }

  async function refreshSourceList(): Promise<void> {
    try {
      await refreshSources();
    } catch (error) {
      notify(`刷新书源失败：${errorText(error)}`, "error");
    }
  }

  async function changeSourceEnabled(source: SourceMetadata, enabled: boolean): Promise<void> {
    if (recoveryRequired.value || sourceBatchBusy.value || sourceExportBusy.value || sourceRemovalBusy.value || sourceEditBusy.value) return;
    const revision = getRestoreFrontendRevision();
    try {
      await saveSourceMetadata(source, { enabled });
      if (revision !== getRestoreFrontendRevision()) return;
      if (!enabled || source.isRss === true) selectedSourceIds.value = selectedSourceIds.value.filter((id) => id !== source.id);
      else if (!selectedSourceIds.value.includes(source.id)) selectedSourceIds.value.push(source.id);
      notify(`${source.name} 已${enabled ? "启用" : "停用"}。`);
    } catch (error) {
      if (revision !== getRestoreFrontendRevision()) return;
      notify(`更新书源状态失败：${errorText(error)}`, "error");
      await refreshSources().catch((refreshError) => notify(errorText(refreshError), "error"));
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
        await saveSourceMetadata(source, normalizedPatch);
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
    let refreshed = false;
    if (failure) {
      try {
        await refreshSources();
        refreshed = true;
      } catch {
        // 恢复读取失败时保留最后一次已确认的书源快照。
      }
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
      if (refreshed) {
        const nowMatching = targets.filter((target) => {
          const latest = sources.value.find((source) => source.id === target.id);
          return latest && matchesPatch(latest);
        }).length;
        notify(`状态已重新读取：${nowMatching}/${targets.length} 个书源已达到目标状态；批量操作中断：${errorText(failure)}`, "error");
      } else {
        notify(`批量操作中断，已确认完成 ${completed}/${targets.length} 个书源；重新读取状态失败：${errorText(failure)}`, "error");
      }
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
      const definition: Record<string, unknown> = JSON.parse(json);
      sourceEditJson.value = json;
      sourceEditOriginalJson.value = json;
      sourceEditInitialUrl.value = String(definition.bookSourceUrl ?? definition.url ?? definition.sourceUrl ?? "");
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
      const result = await mutateSource({
        kind: "save",
        id: source.id,
        json: definitionJson,
      });
      if (revision !== getRestoreFrontendRevision()) return;
      await refreshSources(sourceResource(result));
      editingSource.value = null;
      sourceEditJson.value = "";
      sourceEditOriginalJson.value = "";
      notify("完整书源 JSON 已保存，新规则会用于后续搜索和解析。");
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
    // Rust owns source IDs, validation, and persisted source snapshots.
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
      const response = await mutateSource({
        kind: "save",
        id: null,
        json: sourceJson,
      });
      if (restoreRevision !== getRestoreFrontendRevision()) return false;
      await refreshSources(sourceResource(response));
      sourceImportOpen.value = false;
      notify("来源配置已保存。可在内容引擎中继续编辑所有字段和解析规则。");
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
      await refreshSources(sourceResource(response));
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
      const response = await mutateSource({
        kind: "url",
        url,
      });
      if (revision !== getRestoreFrontendRevision()) return;
      await refreshSources(sourceResource(response));
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
      const descriptor = sourceDefinitionsResource.value;
      if (!descriptor) throw new Error("书源资源尚未加载，无法导出");
      const document = await readResource<SourceDefinitionsResource>(descriptor);
      const requestedIds = new Set(sourceIds);
      const definitions = document.sources.filter((source) => requestedIds.has(source.id));
      if (definitions.length !== requestedIds.size) {
        throw new Error("部分所选书源已不存在，请刷新列表后重试");
      }
      const sourceJson = definitions.map(({ definitionJson }) => {
        // Keep each Rust-serialized JSON object as text so 64-bit numeric
        // fields are not rounded by JavaScript's number representation.
        JSON.parse(definitionJson);
        return definitionJson.split(/\r?\n/).map((line) => `  ${line}`).join("\n");
      }).join(",\n");
      const json = `[\n${sourceJson}\n]\n`;
      const url = URL.createObjectURL(new Blob([json], { type: "application/json;charset=utf-8" }));
      const anchor = window.document.createElement("a");
      anchor.href = url;
      anchor.download = "book-sources.json";
      window.document.body.append(anchor);
      try {
        anchor.click();
        notify(`已导出 ${definitions.length} 个书源。`);
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
      const resource = sourceResource(result);
      // Rust has committed removal. Do not leave a confirm dialog that can
      // accidentally submit the same destructive operation a second time.
      const removed = new Set(targets);
      selectedSourceRows.value = selectedSourceRows.value.filter((id) => !removed.has(id));
      pendingSourceRemoval.value = [];
      try {
        await refreshSources(resource);
      } catch (error) {
        if (revision === getRestoreFrontendRevision()) {
          notify(`书源已移除，但重新读取列表失败：${errorText(error)}`, "error");
        }
        return;
      }
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
    refreshSources,
    refreshSourceList,
    changeSourceEnabled,
    patchSelectedSources,
    setSelectedSourcesEnabled,
    setSelectedSourcesGroup,
    beginSourceEdit,
    closeSourceEdit,
    saveSourceDefinitionJson,
    sourceEditJson,
    sourceEditOriginalJson,
    sourceEditInitialUrl,
    importSourceFileFromPicker,
    importSourceFromUrl,
    createSourceFromJson,
    exportSelectedSources,
    deleteSelectedSources,
  };
}

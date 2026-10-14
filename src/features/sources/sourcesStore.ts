import { computed, shallowRef } from "vue";
import { listen } from "@tauri-apps/api/event";
import { mutateSource as mutateSourceCommand } from "../../api/sources";
import { readResource } from "../../api/resources";
import { removeSourceJsonValue, setSourceJsonValue } from "./sourceJsonEdits";
import type { AppBootstrap, ResourceDescriptor, SourceDefinitionResource, SourceListResource, SourceMetadata, SourceMutation, SourceMutationResponse } from "../../api/types";

export const sources = shallowRef<SourceMetadata[]>([]);
export const enabledSources = computed(() => sources.value.filter(source => source.enabled));
export const bookSourceCandidates = computed(() => enabledSources.value.filter(source => source.isRss !== true && source.searchType !== undefined));

export type SourceMetadataChanges = { enabled?: boolean; group?: string | null };

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

type SourceListUpdatedEvent = { requestId: number; resource: ResourceDescriptor };
let root: ResourceDescriptor;
let readRevision = 0;
let requestId = 0;
// A mutation completes after its list notification has been applied. Full
// definitions stay in the caller's editor draft, never in this shared store.
const pendingUpdates = new Map<number, { resolve: () => void; reject: (error: unknown) => void }>();

async function updateSources(resource: ResourceDescriptor): Promise<void> {
  const revision = ++readRevision;
  root = resource;
  const document = await readResource<SourceListResource>(resource);
  if (document.schemaVersion !== 1 || !Array.isArray(document.sources)) throw new Error("书源列表格式无效。");
  if (revision === readRevision) sources.value = document.sources;
}

export async function loadSources(resource: ResourceDescriptor): Promise<void> {
  await listen<SourceListUpdatedEvent>("sources-updated", ({ payload }) => {
    void updateSources(payload.resource).then(
      () => pendingUpdates.get(payload.requestId)?.resolve(),
      error => { pendingUpdates.get(payload.requestId)?.reject(error); console.error(error); },
    );
  });
  await listen<AppBootstrap>("app-state-updated", ({ payload }) => {
    void updateSources(payload.sources).catch(console.error);
  });
  await updateSources(resource);
}

export async function readSourceDefinitionJson(sourceId: string): Promise<string> {
  if (!sources.value.some(source => source.id === sourceId)) throw new Error("书源已不存在。");
  const url = new URL(`sources/${encodeURIComponent(sourceId)}.json`, root.src).href;
  const document = await readResource<SourceDefinitionResource>(url);
  if (document.schemaVersion !== 1 || document.id !== sourceId || typeof document.definitionJson !== "string") {
    throw new Error("书源定义格式无效。");
  }
  return document.definitionJson;
}

export async function mutateSources(mutation: SourceMutation): Promise<SourceMutationResponse> {
  const id = ++requestId;
  const updated = new Promise<void>((resolve, reject) => pendingUpdates.set(id, { resolve, reject }));
  // An event read can fail before the command response reaches the WebView.
  void updated.catch(() => {});
  try {
    const result = await mutateSourceCommand(mutation, id);
    if ("resource" in result && result.resource) await updated;
    return result;
  } finally {
    pendingUpdates.delete(id);
  }
}

export const saveSourceDefinition = (id: string | null, json: string) =>
  mutateSources({ kind: "save", id, json });

export async function exportSourceDefinitions(sourceIds: string[]): Promise<string> {
  const definitions: string[] = [];
  for (const id of sourceIds) {
    const json = await readSourceDefinitionJson(id);
    definitions.push(json.split(/\r?\n/).map(line => `  ${line}`).join("\n"));
  }
  return `[\n${definitions.join(",\n")}\n]\n`;
}

export async function saveSourceMetadata(sourceId: string, changes: SourceMetadataChanges): Promise<void> {
  const json = await readSourceDefinitionJson(sourceId);
  await saveSourceDefinition(sourceId, applySourceMetadataChanges(json, changes));
}

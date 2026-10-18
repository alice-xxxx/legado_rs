import { computed, shallowRef } from "vue";
import { listen } from "@tauri-apps/api/event";
import { mutateSource as mutateSourceCommand } from "../../api/sources";
import { readSources, readSourceDefinition } from "../../api/database";
import { removeSourceJsonValue, setSourceJsonValue } from "./sourceJsonEdits";
import type { AppBootstrap, SourceMetadata, SourceMutation, SourceMutationResponse } from "../../api/types";

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
    throw new Error("源 JSON 必须是一个对象");
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

type SourceListUpdatedEvent = { requestId: number };
let readRevision = 0;
let requestId = 0;
// A mutation completes after its list notification has been applied. Full
// definitions stay in the caller's editor draft, never in this shared store.
const pendingUpdates = new Map<number, { resolve: () => void; reject: (error: unknown) => void }>();

async function updateSources(): Promise<void> {
  const revision = ++readRevision;
  const document = { schemaVersion: 1, sources: await readSources() };
  if (document.schemaVersion !== 1 || !Array.isArray(document.sources)) throw new Error("源列表格式无效。");
  if (revision === readRevision) sources.value = document.sources;
}

export async function loadSources(): Promise<void> {
  await listen<SourceListUpdatedEvent>("sources-updated", ({ payload }) => {
    void updateSources().then(
      () => pendingUpdates.get(payload.requestId)?.resolve(),
      error => { pendingUpdates.get(payload.requestId)?.reject(error); console.error(error); },
    );
  });
  await listen<AppBootstrap>("app-state-updated", () => {
    void updateSources().catch(console.error);
  });
  await updateSources();
}

export async function readSourceDefinitionJson(sourceId: string): Promise<string> {
  if (!sources.value.some(source => source.id === sourceId)) throw new Error("源已不存在。");
  return readSourceDefinition(sourceId);
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

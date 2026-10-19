import { computed, shallowRef } from "vue";
import { listen } from "@tauri-apps/api/event";
import { modifyDatabase } from "../../api/feDb";
import { prepareConfigurationImport, type ConfigurationImportSource } from "../../api/settings";
import { readSources, readSourceDefinition } from "../../api/database";
import { removeSourceJsonValue, setSourceJsonValue } from "./sourceJsonEdits";
import type { AppBootstrap, SourceMetadata } from "../../api/types";

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

let readRevision = 0;

async function updateSources(): Promise<void> {
  const revision = ++readRevision;
  const next = await readSources();
  if (revision === readRevision) sources.value = next;
}

export async function loadSources(): Promise<void> {
  await listen("sources-updated", () => { void updateSources().catch(console.error); });
  await listen<AppBootstrap>("app-state-updated", () => {
    void updateSources().catch(console.error);
  });
  await updateSources();
}

export async function readSourceDefinitionJson(sourceId: string): Promise<string> {
  if (!sources.value.some(source => source.id === sourceId)) throw new Error("源已不存在。");
  return readSourceDefinition(sourceId);
}

export async function importSourceDefinitions(definitions: string[]): Promise<void> {
  await modifyDatabase(definitions.map(json => ({ action: "upsert", table: "sources", keys: ["id"], values: { definition_json: json } })))
    .finally(updateSources);
}

export async function saveSourceDefinition(id: string | null, json: string): Promise<void> {
  // 原始 JSON 按文本传递，避免 Vue 解析后改写源内的 64 位整数。
  await modifyDatabase([{ action: "upsert", table: "sources", keys: ["id"], values: { ...(id ? { id } : {}), definition_json: json } }])
    .finally(updateSources);
}

export async function importSourceDefinitionsFrom(source: ConfigurationImportSource): Promise<boolean> {
  const document = await prepareConfigurationImport<{ sources: string[] }>("sources", source);
  if (!document) return false;
  await importSourceDefinitions(document.sources);
  return true;
}

export async function removeSources(sourceIds: string[]): Promise<void> {
  await modifyDatabase([{ action: "delete", table: "sources", filter: { id: sourceIds } }]).finally(updateSources);
}

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

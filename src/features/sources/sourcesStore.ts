import { computed, shallowRef } from "vue";
import { listen } from "@tauri-apps/api/event";
import { modifyDatabase } from "../../api/feDb";
import { prepareConfigurationImport, type ConfigurationImportSource } from "../../api/settings";
import { readSources, readSourceDefinition } from "../../api/database";
import type { AppBootstrap, SourceMetadata } from "../../api/types";

export const sources = shallowRef<SourceMetadata[]>([]);
export const enabledSources = computed(() => sources.value.filter(source => source.enabled));
export const bookSourceCandidates = computed(() => enabledSources.value.filter(source => source.isRss !== true && source.searchType !== undefined));

export type SourceMetadataChanges = { enabled?: boolean; group?: string | null };

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
  await modifyDatabase([{ operation: "saveSources", sources: definitions.map(definition => ({ definition })) }], { wait: true })
    .finally(updateSources);
}

export async function saveSourceDefinition(id: string | null, json: string): Promise<void> {
  // 原始 JSON 按文本传递，避免 Vue 解析后改写源内的 64 位整数。
  await modifyDatabase([{ operation: "saveSources", sources: [{ ...(id ? { id } : {}), definition: json }] }], { wait: true })
    .finally(updateSources);
}

export async function importSourceDefinitionsFrom(source: ConfigurationImportSource) {
  const document = await prepareConfigurationImport<{ sources: string[]; errors: string[] }>("sources", source);
  if (!document) return null;
  await importSourceDefinitions(document.sources);
  return { count: document.sources.length, errors: document.errors };
}

export async function removeSources(sourceIds: string[]): Promise<void> {
  await modifyDatabase([{ operation: "saveSources", sources: sourceIds.map(id => ({ id, removed: true })) }], { wait: true }).finally(updateSources);
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
  await modifyDatabase([{ operation: "saveSources", sources: [{ id: sourceId, ...changes }] }], { wait: true }).finally(updateSources);
}

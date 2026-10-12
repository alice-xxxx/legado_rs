import type { Ref } from "vue";
import { prepareConfigurationImport } from "../../api/settings";
import { httpTtsStore } from "./configurationStores";
import { settings } from "./settingsStore";
import { type HttpTtsConfigEditorFields, type HttpTtsConfigEntry, type HttpTtsConfigMetadata, type HttpTtsConfigResource } from "../../api/types";

interface HttpTtsConfigActionOptions {
  error: Ref<string>;
  importBusy: Ref<boolean>;
  selectConfig: (id: string) => void;
  getRestoreRevision: () => number;
  notify: (message: string, kind?: "success" | "error") => void;
  errorText: (error: unknown) => string;
}

/** The form edits a draft; save writes one complete entry to the shared state. */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function optionalText(value: unknown): string | null {
  if (value === null || value === undefined) return null;
  if (typeof value === "string" || typeof value === "number" || typeof value === "boolean") return String(value);
  return JSON.stringify(value) ?? null;
}

function editorFields(config: Record<string, unknown>): HttpTtsConfigEditorFields {
  return {
    name: typeof config.name === "string" ? config.name : "",
    url: typeof config.url === "string" ? config.url : "",
    contentType: optionalText(config.contentType),
    concurrentRate: optionalText(config.concurrentRate) ?? "0",
    loginUrl: optionalText(config.loginUrl),
    loginUi: optionalText(config.loginUi),
    loginCheckJs: optionalText(config.loginCheckJs),
    header: optionalText(config.header),
    jsLib: optionalText(config.jsLib),
    enabledCookieJar: typeof config.enabledCookieJar === "boolean" ? config.enabledCookieJar : null,
    enableDangerousApi: typeof config.enableDangerousApi === "boolean" ? config.enableDangerousApi : null,
  };
}

type TextFieldKey = "contentType" | "loginUrl" | "loginCheckJs" | "jsLib";

function setTextField(config: Record<string, unknown>, key: TextFieldKey, value: string | null): void {
  const previous = optionalText(config[key]) ?? "";
  if (previous === (value ?? "")) return;
  const cleaned = value?.trim();
  if (cleaned) config[key] = cleaned;
  else delete config[key];
}

function setRateField(config: Record<string, unknown>, value: string | null, isNew: boolean): void {
  if (isNew) {
    config.concurrentRate = value?.trim() || "0";
    return;
  }
  const previous = optionalText(config.concurrentRate) ?? "0";
  if (previous === (value ?? "0")) return;
  const cleaned = value?.trim();
  if (cleaned) config.concurrentRate = cleaned;
  else delete config.concurrentRate;
}

function setBooleanField(config: Record<string, unknown>, key: string, value: boolean | null): void {
  const previous = typeof config[key] === "boolean" ? config[key] as boolean : null;
  if (previous === value) return;
  if (value === null) delete config[key];
  else config[key] = value;
}

function setStructuredField(config: Record<string, unknown>, key: "header" | "loginUi", value: string | null): void {
  const original = config[key];
  const previous = optionalText(original) ?? "";
  const text = value ?? "";
  if (previous === text) return;
  if (!text) {
    delete config[key];
    return;
  }

  const mustKeepType = isRecord(original) || Array.isArray(original)
    || typeof original === "number" || typeof original === "boolean";
  if (mustKeepType) {
    let parsed: unknown;
    try {
      parsed = JSON.parse(text);
    } catch (error) {
      throw new Error(`${key} 必须保留原 JSON 类型：${String(error)}`);
    }
    const sameType = isRecord(original) ? isRecord(parsed)
      : Array.isArray(original) ? Array.isArray(parsed)
        : typeof original === typeof parsed;
    if (!sameType) throw new Error(`${key} 必须保留原 JSON 类型，请输入对应的 JSON 值`);
    config[key] = parsed;
    return;
  }

  if (typeof original === "string") {
    config[key] = text;
    return;
  }
  const trimmed = text.trim();
  if (trimmed.startsWith("{") || trimmed.startsWith("[")) {
    try {
      const parsed: unknown = JSON.parse(trimmed);
      if (isRecord(parsed) || Array.isArray(parsed)) {
        config[key] = parsed;
        return;
      }
    } catch {
      // New text fields may contain literal Legado strings that resemble JSON.
    }
  }
  config[key] = text;
}

function editorConfigJson(
  configId: string | null,
  fields: HttpTtsConfigEditorFields,
  entries: HttpTtsConfigEntry[],
): string {
  const entry = configId ? entries.find((item) => item.id === configId) : undefined;
  if (configId && !entry) throw new Error("当前配置已从资源中移除，请重新载入后再保存");
  const config: Record<string, unknown> = entry ? { ...entry.config } : {};
  config.name = fields.name.trim();
  config.url = fields.url.trim();
  setTextField(config, "contentType", fields.contentType);
  setRateField(config, fields.concurrentRate, !entry);
  setTextField(config, "loginUrl", fields.loginUrl);
  setStructuredField(config, "loginUi", fields.loginUi);
  setTextField(config, "loginCheckJs", fields.loginCheckJs);
  setStructuredField(config, "header", fields.header);
  setTextField(config, "jsLib", fields.jsLib);
  setBooleanField(config, "enabledCookieJar", fields.enabledCookieJar);
  setBooleanField(config, "enableDangerousApi", fields.enableDangerousApi);
  return JSON.stringify(config);
}

export function useHttpTtsConfigActions(options: HttpTtsConfigActionOptions) {
  function loadHttpTtsConfigEditorFields(configId: string): HttpTtsConfigEditorFields | null {
    const entry = httpTtsStore.items.value.find(item => item.id === configId);
    return entry ? editorFields(entry.config) : null;
  }

  function saveHttpTtsConfigFromEditor(configId: string | null, fields: HttpTtsConfigEditorFields): boolean {
    options.error.value = "";
    try {
      const config = JSON.parse(editorConfigJson(configId, fields, httpTtsStore.items.value)) as Record<string, unknown>;
      const id = configId ?? crypto.randomUUID().replace(/-/g, "");
      const enabled = configId ? httpTtsStore.items.value.find(item => item.id === configId)!.enabled : true;
      const entry: HttpTtsConfigEntry = { id, name: String(config.name), enabled, config };
      httpTtsStore.update(entry);
      if (!configId) options.selectConfig(id);
      options.notify("HTTP TTS 配置已更新。");
      return true;
    } catch (error) {
      options.error.value = options.errorText(error);
      return false;
    }
  }

  function applyImportedItems(items: HttpTtsConfigMetadata[]): void {
    const imported = items.filter((item) => typeof item.id === "string" && typeof item.name === "string");
    if (!imported.length) return;
    options.selectConfig(imported[0].id);
    options.notify(imported.length === 1
      ? `已导入 HTTP TTS 配置“${imported[0].name}”。`
      : `已导入 ${imported.length} 个 HTTP TTS 配置。`);
  }

  async function importHttpTtsConfigFromLocal(): Promise<boolean> {
    if (options.importBusy.value) return false;
    const revision = options.getRestoreRevision();
    const isCurrent = () => revision === options.getRestoreRevision();
    options.importBusy.value = true;
    options.error.value = "";
    try {
      const result = await prepareConfigurationImport<HttpTtsConfigResource>("httpTtsConfigs", { kind: "local" });
      if (!result || !isCurrent()) return false;
      result.configs.forEach(entry => httpTtsStore.update(entry));
      applyImportedItems(result.configs);
      return true;
    } catch (error) {
      if (isCurrent()) options.error.value = `本地导入 HTTP TTS 配置失败：${options.errorText(error)}`;
      return false;
    } finally {
      if (isCurrent()) options.importBusy.value = false;
    }
  }

  async function importHttpTtsConfigFromLink(url: string): Promise<boolean> {
    const sourceUrl = url.trim();
    if (!sourceUrl || options.importBusy.value) return false;
    const revision = options.getRestoreRevision();
    const isCurrent = () => revision === options.getRestoreRevision();
    options.importBusy.value = true;
    options.error.value = "";
    try {
      const result = await prepareConfigurationImport<HttpTtsConfigResource>("httpTtsConfigs", { kind: "url", url: sourceUrl });
      if (!result || !isCurrent()) return false;
      result.configs.forEach(entry => httpTtsStore.update(entry));
      applyImportedItems(result.configs);
      return true;
    } catch (error) {
      if (isCurrent()) options.error.value = `从链接导入 HTTP TTS 配置失败：${options.errorText(error)}`;
      return false;
    } finally {
      if (isCurrent()) options.importBusy.value = false;
    }
  }

  function removeHttpTtsConfigs(configIds: string[]): void {
    configIds.forEach(id => httpTtsStore.remove(id));
    if (configIds.includes(settings.value.reader.httpTtsConfigId ?? "")) options.selectConfig("");
    options.notify("HTTP TTS 配置已移除。");
  }


  function removeSelectedHttpTtsConfig(configIds: string[]): void {
    removeHttpTtsConfigs(configIds);
  }

  function resetForRestore(): void {
    options.importBusy.value = false;
    options.error.value = "";
  }

  function exportHttpTtsConfig(configIds: string | string[]): void {
    const ids = typeof configIds === "string" ? [configIds] : [...new Set(configIds)];
    const entries = ids.map((id) => httpTtsStore.items.value.find((item) => item.id === id));
    if (!entries.length || entries.some((entry) => !entry)) {
      options.error.value = "HTTP TTS 配置尚未加载，无法导出。";
      return;
    }
    const validEntries = entries as HttpTtsConfigEntry[];
    options.error.value = "";
    const payload = validEntries.length === 1 ? validEntries[0].config : validEntries.map((entry) => entry.config);
    const blob = new Blob([JSON.stringify(payload, null, 2)], { type: "application/json;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const anchor = window.document.createElement("a");
    let safeName = (validEntries.length === 1 ? validEntries[0].name : `http-tts-configs-${validEntries.length}`)
      .replace(/[<>:"/\\|?*\u0000-\u001f]/g, "_").replace(/[. ]+$/g, "").trim();
    if (/^(con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/i.test(safeName)) safeName = `_${safeName}`;
    safeName = Array.from(safeName || "http-tts").slice(0, 120).join("");
    anchor.href = url;
    anchor.download = `${safeName}.json`;
    window.document.body.append(anchor);
    anchor.click();
    anchor.remove();
    window.setTimeout(() => URL.revokeObjectURL(url), 1000);
  }

  return {
    resetForRestore,
    loadHttpTtsConfigEditorFields,
    exportHttpTtsConfig,
    saveHttpTtsConfigFromEditor,
    importHttpTtsConfigFromLocal,
    importHttpTtsConfigFromLink,
    removeSelectedHttpTtsConfig,
  };
}

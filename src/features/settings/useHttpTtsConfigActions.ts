import type { Ref } from "vue";
import { mutateHttpTtsConfigs } from "../../api/settings";
import { readResource } from "../../api/resources";
import { type AppSettingsResource, type HttpTtsConfigEditorFields, type HttpTtsConfigEntry, type HttpTtsConfigMetadata, type HttpTtsConfigResource, type ResourceDescriptor } from "../../api/types";
import { beginReaderHttpTtsConfigRefresh, publishReaderHttpTtsConfigs } from "../reader/readerTtsConfigService";

interface HttpTtsConfigActionOptions {
  configs: Ref<HttpTtsConfigMetadata[]>;
  busy: Ref<boolean>;
  error: Ref<string>;
  importBusy: Ref<boolean>;
  editBusy: Ref<boolean>;
  deleteBusyId: Ref<string | null>;
  resource: Ref<ResourceDescriptor | null>;
  settings: Ref<AppSettingsResource>;
  selectConfig: (id: string) => void;
  getRestoreRevision: () => number;
  notify: (message: string, kind?: "success" | "error") => void;
  errorText: (error: unknown) => string;
}

/** Read/export the resource and serialize form edits before calling Rust's JSON save command. */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isConfigEntry(value: unknown): value is HttpTtsConfigEntry {
  return isRecord(value)
    && typeof value.id === "string"
    && typeof value.name === "string"
    && typeof value.enabled === "boolean"
    && isRecord(value.config);
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
  let refreshGeneration = 0;
  let document: HttpTtsConfigResource | null = null;

  async function refreshHttpTtsConfigs(resource?: ResourceDescriptor): Promise<void> {
    if (resource) options.resource.value = resource;
    const generation = ++refreshGeneration;
    const readerGeneration = beginReaderHttpTtsConfigRefresh();
    const restoreRevision = options.getRestoreRevision();
    const isCurrent = () => generation === refreshGeneration && restoreRevision === options.getRestoreRevision();
    const descriptor = options.resource.value;
    options.busy.value = true;
    options.error.value = "";
    try {
      if (!descriptor) throw new Error("HTTP TTS 配置资源尚未就绪");
      const response = await readResource<HttpTtsConfigResource>(descriptor);
      if (!response || !Array.isArray(response.configs)) throw new Error("HTTP TTS 配置资源格式无效");
      if (!isCurrent()) return;
      document = { ...response, configs: response.configs.filter(isConfigEntry) };
      options.configs.value = document.configs.map(({ id, name, enabled }) => ({ id, name, enabled }));
      publishReaderHttpTtsConfigs(readerGeneration, options.configs.value);
      const selectedId = options.settings.value.reader.httpTtsConfigId;
      if (selectedId && !options.configs.value.some((item) => item.id === selectedId && item.enabled)) {
        options.selectConfig("");
      }
    } catch (error) {
      if (isCurrent()) options.error.value = `读取 HTTP TTS 配置失败：${options.errorText(error)}`;
    } finally {
      // An obsolete restore-era read still owns this loading indicator until a newer read starts.
      if (isCurrent()) options.busy.value = false;
    }
  }

  async function loadHttpTtsConfigEditorFields(configId: string): Promise<HttpTtsConfigEditorFields | null> {
    if (!configId || options.editBusy.value) return null;
    const revision = options.getRestoreRevision();
    const isCurrent = () => revision === options.getRestoreRevision();
    options.editBusy.value = true;
    options.error.value = "";
    try {
      await refreshHttpTtsConfigs();
      if (!isCurrent() || options.error.value) return null;
      const entry = document?.configs.find((item) => item.id === configId);
      return isCurrent() && entry ? editorFields(entry.config) : null;
    } catch (error) {
      if (isCurrent()) options.error.value = `读取 HTTP TTS 配置失败：${options.errorText(error)}`;
      return null;
    } finally {
      if (isCurrent()) options.editBusy.value = false;
    }
  }

  async function saveHttpTtsConfigFromEditor(
    configId: string | null,
    fields: HttpTtsConfigEditorFields,
  ): Promise<boolean> {
    if (options.editBusy.value) return false;
    const revision = options.getRestoreRevision();
    const isCurrent = () => revision === options.getRestoreRevision();
    options.editBusy.value = true;
    options.error.value = "";
    try {
      const configJson = editorConfigJson(configId, fields, document?.configs ?? []);
      const saved = await mutateHttpTtsConfigs({ kind: "save", id: configId, json: configJson });
      if ("cancelled" in saved) return false;
      if (!isCurrent()) return false;
      await refreshHttpTtsConfigs(saved.resource);
      if (!isCurrent()) return false;
      const item = saved.items[0];
      if (!item) throw new Error("保存没有返回 HTTP TTS 配置");
      if (!configId) options.selectConfig(item.id);
      options.notify(`${configId ? "已保存" : "已创建"} HTTP TTS 配置“${item.name}”。`);
      return true;
    } catch (error) {
      if (isCurrent()) options.error.value = `保存 HTTP TTS 配置失败：${options.errorText(error)}`;
      return false;
    } finally {
      if (isCurrent()) options.editBusy.value = false;
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
      const result = await mutateHttpTtsConfigs({ kind: "local" });
      if ("cancelled" in result) return false;
      if (!isCurrent()) return false;
      await refreshHttpTtsConfigs(result.resource);
      if (!isCurrent()) return false;
      applyImportedItems(result.items);
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
      const result = await mutateHttpTtsConfigs({ kind: "url", url: sourceUrl });
      if ("cancelled" in result) return false;
      if (!isCurrent()) return false;
      await refreshHttpTtsConfigs(result.resource);
      if (!isCurrent()) return false;
      applyImportedItems(result.items);
      return true;
    } catch (error) {
      if (isCurrent()) options.error.value = `从链接导入 HTTP TTS 配置失败：${options.errorText(error)}`;
      return false;
    } finally {
      if (isCurrent()) options.importBusy.value = false;
    }
  }

  async function removeHttpTtsConfigs(configIds: string[]): Promise<void> {
    if (options.deleteBusyId.value || options.importBusy.value || options.editBusy.value) return;
    const ids = [...new Set(configIds)];
    const targets = options.configs.value.filter((item) => ids.includes(item.id));
    if (!targets.length || targets.length !== ids.length) return;
    const revision = options.getRestoreRevision();
    const isCurrent = () => revision === options.getRestoreRevision();
    options.deleteBusyId.value = targets[0].id;
    options.error.value = "";
    try {
      const result = await mutateHttpTtsConfigs({ kind: "delete", configIds: ids });
      if ("cancelled" in result) return;
      if (!isCurrent()) return;
      const activeConfigId = options.settings.value.reader.httpTtsConfigId;
      if (result.deleted && activeConfigId && ids.includes(activeConfigId)) {
        options.selectConfig("");
      }
      await refreshHttpTtsConfigs(result.resource);
      if (isCurrent() && result.deleted) {
        options.notify(targets.length === 1
          ? `已删除 HTTP TTS 配置“${targets[0].name}”。`
          : `已删除 ${targets.length} 个 HTTP TTS 配置。`);
      }
    } catch (error) {
      if (isCurrent()) options.error.value = `删除 HTTP TTS 配置失败：${options.errorText(error)}`;
    } finally {
      if (isCurrent()) options.deleteBusyId.value = null;
    }
  }

  function removeHttpTtsConfig(item: HttpTtsConfigMetadata): Promise<void> {
    return removeHttpTtsConfigs([item.id]);
  }

  function removeSelectedHttpTtsConfig(configIds: string[]): void {
    // Playback selection is not the editor selection: disabled engines still
    // need to be editable and removable without becoming the active player.
    void removeHttpTtsConfigs(configIds);
  }

  function resetForRestore(): void {
    refreshGeneration += 1;
    document = null;
    options.resource.value = null;
    options.configs.value = [];
    options.busy.value = false;
    options.importBusy.value = false;
    options.editBusy.value = false;
    options.deleteBusyId.value = null;
    options.error.value = "";
  }

  function exportHttpTtsConfig(configIds: string | string[]): void {
    const ids = typeof configIds === "string" ? [configIds] : [...new Set(configIds)];
    const entries = ids.map((id) => document?.configs.find((item) => item.id === id));
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
    refreshHttpTtsConfigs,
    loadHttpTtsConfigEditorFields,
    exportHttpTtsConfig,
    saveHttpTtsConfigFromEditor,
    importHttpTtsConfigFromLocal,
    importHttpTtsConfigFromLink,
    removeHttpTtsConfig,
    removeSelectedHttpTtsConfig,
  };
}

import { command } from "./ipc";
import type { ResourceDescriptor, ReaderBackgroundPickerResponse, HttpTtsAudioResult, HttpTtsStreamStart, HttpTtsStreamStatus, AppSettingsResource } from "./types";

export type { TxtTocRule, TxtTocRulesDocument } from "./types";
export type ConfigurationImportSource = { kind: "local" } | { kind: "url"; url: string } | { kind: "text"; text: string };
export type ConfigurationImportKind = "sources" | "replacementRules" | "txtTocRules" | "httpTtsConfigs";

export async function prepareDeepLinkImport(kind: ConfigurationImportKind | "auto", text: string) {
  return command<{ kind: ConfigurationImportKind; document: Record<string, unknown> }>(
    "prepare_configuration_import", { kind, source: { kind: "text", text } },
  );
}

export interface SaveSettingsResponse {
  requestId: number;
  resource: ResourceDescriptor;
}

export const pickReaderBackgroundImage = () =>
  command<ReaderBackgroundPickerResponse>("pick_reader_background_image");

export const saveSettings = (settings: AppSettingsResource, requestId: number) =>
  command<SaveSettingsResponse>("save_settings", { settings, requestId });

export async function prepareConfigurationImport<T>(
  kind: "replacementRules" | "txtTocRules" | "httpTtsConfigs", source: ConfigurationImportSource,
): Promise<T | null> {
  const result = await command<{ cancelled: true } | { document: T }>(
    "prepare_configuration_import", { kind, source },
  );
  return "cancelled" in result ? null : result.document;
}

export const requestHttpTtsAudio = (configId: string, text: string, speechRate: number) =>
  command<HttpTtsAudioResult>("request_http_tts_audio", { configId, text, speechRate });

export const releaseHttpTtsAudio = (audioId: string) =>
  command<{ released: boolean }>("release_http_tts_audio", { audioId });

export const startHttpTtsStream = (configId: string, chunks: Array<{ text: string; highlightIndex: number }>, speechRate: number) =>
  command<HttpTtsStreamStart>("start_http_tts_stream", { configId, chunks, speechRate });

export const getHttpTtsStreamStatus = (streamId: string) =>
  command<HttpTtsStreamStatus>("get_http_tts_stream_status", { streamId });

export const releaseHttpTtsStream = (streamId: string) =>
  command<boolean>("release_http_tts_stream", { streamId });

import { command } from "./ipc";
import { readResource } from "./resources";
import type { ResourceDescriptor, ReaderBackgroundPickerResponse, HttpTtsAudioResult, AppSettingsResource } from "./types";

export type { TxtTocRule, TxtTocRulesDocument } from "./types";
export type ConfigurationImportSource = { kind: "local" } | { kind: "url"; url: string };

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
  const result = await command<{ cancelled: true } | { resource: ResourceDescriptor }>(
    "prepare_configuration_import", { kind, source },
  );
  return "cancelled" in result ? null : readResource<T>(result.resource);
}

export const requestHttpTtsAudio = (configId: string, text: string, speechRate: number) =>
  command<HttpTtsAudioResult>("request_http_tts_audio", { configId, text, speechRate });

export const releaseHttpTtsAudio = (audioId: string) =>
  command<{ released: boolean }>("release_http_tts_audio", { audioId });

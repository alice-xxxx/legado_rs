import { command } from "./ipc";
import type { ResourceDescriptor, ReaderBackgroundPickerResponse, HttpTtsConfigMutation, HttpTtsConfigMutationResponse, HttpTtsAudioResult, AppSettingsResource, ReplacementRulesMutation, ReplacementRulesMutationResponse, TxtTocRulesMutation, TxtTocRulesMutationResponse } from "./types";

export type { TxtTocRule, TxtTocRulesDocument, TxtTocRulesMutation, TxtTocRulesMutationResponse } from "./types";

export const pickReaderBackgroundImage = () =>
  command<ReaderBackgroundPickerResponse>("pick_reader_background_image");

export const clearReaderBackgroundImage = () =>
  command<ResourceDescriptor>("clear_reader_background_image");

export const saveSettings = (settings: AppSettingsResource) =>
  command<ResourceDescriptor>("save_settings", { settings });

export const mutateHttpTtsConfigs = (mutation: HttpTtsConfigMutation) =>
  command<HttpTtsConfigMutationResponse>("mutate_http_tts_configs", { mutation });

export const mutateTxtTocRules = (mutation: TxtTocRulesMutation) =>
  command<TxtTocRulesMutationResponse>("mutate_txt_toc_rules", { mutation });

export const mutateReplacementRules = (mutation: ReplacementRulesMutation) =>
  command<ReplacementRulesMutationResponse>("mutate_replacement_rules", { mutation });

export const requestHttpTtsAudio = (configId: string, text: string, speechRate: number) =>
  command<HttpTtsAudioResult>("request_http_tts_audio", { configId, text, speechRate });

export const releaseHttpTtsAudio = (audioId: string) =>
  command<{ released: boolean }>("release_http_tts_audio", { audioId });

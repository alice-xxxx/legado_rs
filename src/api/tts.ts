import { command } from "./ipc";

export type TtsProgress = number | string | { paused: boolean };
export interface TtsVoice { voiceURI: string; name: string; lang: string; }
export const ttsVoices = () => command<TtsVoice[]>("tts_voices");

export const startTts = (options: { configId: string | null; voiceId: string | null; rate: number }) =>
  command<void>("start_tts", { options });
export const stopTts = () => command<void>("stop_tts");
export const submitTts = (texts: Array<{ text: string; index: number }>, finished = false) =>
  command<void>("submit_tts", { texts, finished });

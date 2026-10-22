import { command } from "./ipc";

export type TtsProgress = number | string;

export const startTts = (options: { configId: string | null; voiceId: string | null; rate: number }) =>
  command<void>("start_tts", { options });
export const stopTts = () => command<void>("stop_tts");
export const submitTts = (texts: Array<{ text: string; index: number }>) =>
  command<void>("submit_tts", { texts });

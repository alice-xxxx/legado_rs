import { isTauri } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { getVoices, isInitialized } from "tauri-plugin-tts-api";
import { usesNativeAudio } from "./nativeAudio";

export interface TtsVoice { voiceURI: string; name: string; lang: string; }

export async function ttsVoices(): Promise<TtsVoice[]> {
  if (usesNativeAudio()) {
    if (!(await isInitialized()).initialized) return [];
    return (await getVoices()).map(voice => ({ voiceURI: voice.id, name: voice.name, lang: voice.language }));
  }
  return window.speechSynthesis?.getVoices() ?? [];
}

/** 桌面输出适配：只播放 Rust 指定的内容，并回报硬件进度，不持有文本队列。 */
export async function setupTtsOutput(): Promise<() => void> {
  if (!isTauri() || usesNativeAudio()) return () => {};
  let audio: HTMLAudioElement | null = null;
  const utterances = new Set<SpeechSynthesisUtterance>();
  const report = (data: Record<string, unknown>) => { void emit("tts-playback", data); };
  const stop = () => {
    if (audio) {
      audio.onplay = audio.ontimeupdate = audio.onended = audio.onerror = null;
      audio.pause(); audio.removeAttribute("src"); audio.load(); audio = null;
    }
    for (const utterance of utterances) {
      utterance.onstart = utterance.onend = utterance.onerror = null;
    }
    utterances.clear();
    window.speechSynthesis?.cancel();
  };
  const unlisten = await listen<{ action: "stop" | "audio" | "speak"; src: string; text: string; voiceId: string; rate: number; id: string; append: boolean }>("tts-output", ({ payload }) => {
    if (payload.action !== "speak" || !payload.append) stop();
    if (payload.action === "audio") {
      const output = audio = new Audio(payload.src);
      output.onplay = output.ontimeupdate = () => report({ state: "time", position: output.currentTime });
      output.onended = () => report({ state: "ended" });
      output.onerror = () => report({ state: "error", error: output.error?.message || "音频播放失败" });
      void output.play().catch(error => { if (audio === output) report({ state: "error", error: String(error) }); });
    } else if (payload.action === "speak") {
      if (!window.speechSynthesis) { report({ id: payload.id, state: "error", error: "当前环境不支持系统朗读" }); return; }
      const utterance = new SpeechSynthesisUtterance(payload.text);
      utterances.add(utterance);
      utterance.voice = window.speechSynthesis.getVoices().find(voice => voice.voiceURI === payload.voiceId) ?? null;
      utterance.rate = payload.rate;
      utterance.onstart = () => report({ id: payload.id, state: "start" });
      utterance.onend = () => { utterances.delete(utterance); report({ id: payload.id, state: "ended" }); };
      utterance.onerror = event => { utterances.delete(utterance); report({ id: payload.id, state: "error", error: event.error }); };
      window.speechSynthesis.speak(utterance);
    }
  });
  return () => { unlisten(); stop(); };
}

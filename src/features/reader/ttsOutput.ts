import { isTauri } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";

export async function pauseTtsOutput(paused: boolean, http: boolean): Promise<void> {
  if (!http) {
    await emit("tts-playback", { state: paused ? "pause-requested" : "resume-requested" });
  } else await emit("tts-output", { action: paused ? "pause" : "resume" });
}

export async function setTtsOutputRate(rate: number): Promise<void> {
  await emit("tts-output", { action: "rate", rate });
}

/** 各平台共用网页音频输出；文本队列和音频生成仍由 Rust 管理。 */
export async function setupTtsOutput(sync: (media: HTMLAudioElement) => void): Promise<() => void> {
  if (!isTauri()) return () => {};
  const sharedAudio = new Audio();
  let audio: HTMLAudioElement | null = null;
  let outputRate = 1;
  let revision = 0;
  const report = (data: Record<string, unknown>) => { void emit("tts-playback", data); };
  const stop = () => {
    ++revision;
    if (audio) {
      audio.onplay = audio.onpause = audio.ontimeupdate = audio.onended = audio.onerror = audio.onratechange = null;
      audio.pause(); audio.removeAttribute("src"); audio.load(); audio = null;
    }
  };
  const unlisten = await listen<{ action: "stop" | "audio" | "pause" | "resume" | "rate"; src: string; rate: number }>("tts-output", ({ payload }) => {
    if (payload.action === "pause" || payload.action === "resume") {
      if (audio) {
        if (payload.action === "pause") audio.pause();
        else void audio.play().catch(error => report({ state: "error", error: String(error) }));
      }
      return;
    }
    if (payload.action === "rate") { outputRate = payload.rate; if (audio) audio.playbackRate = outputRate; return; }
    if (payload.action === "stop") outputRate = 1;
    stop();
    if (payload.action === "audio") {
      const output = audio = sharedAudio;
      const current = revision;
      output.src = payload.src;
      output.playbackRate = outputRate;
      output.onplay = output.ontimeupdate = () => {
        sync(output);
        report({ state: "time", position: output.currentTime });
      };
      output.onpause = () => {
        sync(output);
        if (!output.ended) report({ state: "paused" });
      };
      output.onended = () => { sync(output); report({ state: "ended" }); };
      output.onratechange = () => sync(output);
      output.onerror = () => report({ state: "error", error: output.error?.message || "音频播放失败" });
      void output.play().catch(error => { if (current === revision) report({ state: "error", error: String(error) }); });
    }
  });
  return () => { unlisten(); stop(); };
}

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
  let playAttempt = 0;
  const report = (data: Record<string, unknown>) => { void emit("tts-playback", data); };
  const play = () => {
    const output = audio;
    if (!output) return;
    const current = revision, attempt = ++playAttempt;
    void output.play().catch(error => {
      if (current === revision && attempt === playAttempt) report({ state: "error", error: String(error) });
    });
  };
  const stop = (release = true) => {
    ++revision;
    if (audio) {
      audio.onplay = audio.onplaying = audio.onwaiting = audio.onstalled = audio.onpause = audio.ontimeupdate = audio.onended = audio.onerror = audio.onratechange = null;
      audio.pause();
      if (release) { audio.removeAttribute("src"); audio.load(); }
      audio = null;
    }
  };
  const unlisten = await listen<{ action: "stop" | "audio" | "pause" | "resume" | "rate"; src: string; rate: number }>("tts-output", ({ payload }) => {
    if (payload.action === "pause" || payload.action === "resume") {
      if (audio) {
        if (payload.action === "pause") { ++playAttempt; audio.pause(); }
        else play();
      }
      return;
    }
    if (payload.action === "rate") { outputRate = payload.rate; if (audio) audio.playbackRate = outputRate; return; }
    if (payload.action === "stop") outputRate = 1;
    stop(payload.action !== "audio");
    if (payload.action === "audio") {
      const output = audio = sharedAudio;
      const current = revision;
      const valid = () => audio === output && current === revision;
      output.src = payload.src;
      output.playbackRate = outputRate;
      output.onplay = output.ontimeupdate = () => {
        if (!valid() || output.paused || output.ended) return;
        sync(output);
        report({ state: "time", position: output.currentTime });
      };
      output.onpause = () => {
        if (!valid() || !output.paused || output.ended) return;
        sync(output);
        report({ state: "paused" });
      };
      output.onwaiting = output.onstalled = () => {
        if (valid() && !output.paused && !output.ended) report({ state: "buffering" });
      };
      output.onplaying = () => {
        if (valid() && !output.paused && !output.ended) report({ state: "playing" });
      };
      // 只上报实际播放状态；音频预取和拼接由 Rust 独立推进。
      output.onended = () => { if (valid() && output.ended) report({ state: "ended" }); };
      output.onratechange = () => { if (valid() && !output.ended) sync(output); };
      output.onerror = () => {
        if (valid() && output.error) report({ state: "error", error: output.error.message || "音频播放失败" });
      };
      play();
    }
  });
  return () => { unlisten(); stop(); };
}

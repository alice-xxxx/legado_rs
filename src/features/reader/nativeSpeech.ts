import * as tts from "tauri-plugin-tts-api";
import type { SpeechEvent, SpeechEventType, Voice } from "tauri-plugin-tts-api";
import { disposeNativeAudio, registerBeforeNativeAudioStart } from "./nativeAudio";

export interface NativeSpeechCallbacks {
  start: (index: number) => void;
  paused: () => void;
  resumed: () => void;
  finished: () => void;
  error: (error: unknown) => void;
  notice?: (message: string) => void;
}
interface SpeechOptions { voiceId: string; rate: number; }

/** Queue a chapter in the OS, rather than relying on WebView timers per sentence. */
export class NativeSpeechQueue {
  private revision = 0;
  private commands: Promise<unknown> = Promise.resolve();
  private listeners: Array<() => void> = [];
  private ready: Promise<void> | null = null;
  private ids = new Map<string, number>();
  private early = new Map<string, Array<[SpeechEventType, SpeechEvent]>>();
  private chunks: string[] = [];
  private options: SpeechOptions = { voiceId: "", rate: 1 };
  private callbacks: NativeSpeechCallbacks | null = null;
  private current = 0;
  private enqueuing = false;
  private fullyQueued = false;
  private paused = false;
  private destroyed = false;
  private unregisterAudioStart = registerBeforeNativeAudioStart(() => this.stop());

  private serialize<T>(operation: () => Promise<T>): Promise<T> {
    const result = this.commands.catch(() => undefined).then(operation);
    this.commands = result;
    return result;
  }
  private initialize(): Promise<void> {
    this.ready ??= (async () => {
      await tts.setBackgroundBehavior({ continueInBackground: true });
      const types: SpeechEventType[] = ["speech:start", "speech:finish", "speech:cancel", "speech:error",
        "speech:pause", "speech:resume", "speech:interrupted", "speech:backgroundPause"];
      for (const type of types) {
        const unlisten = await tts.onSpeechEvent(type, event => this.event(type, event));
        if (this.destroyed) unlisten();
        else this.listeners.push(unlisten);
      }
    })().catch(error => {
      this.listeners.splice(0).forEach(unlisten => unlisten());
      this.ready = null;
      throw error;
    });
    return this.ready;
  }
  async voices(): Promise<Voice[]> {
    await this.initialize();
    for (let attempt = 0; attempt < 20 && !this.destroyed; attempt++) {
      const status = await tts.isInitialized();
      if (status.initialized) return tts.getVoices();
      await new Promise(resolve => setTimeout(resolve, 250));
    }
    return [];
  }
  async start(chunks: string[], options: SpeechOptions, callbacks: NativeSpeechCallbacks, from = 0): Promise<void> {
    const revision = ++this.revision;
    this.chunks = chunks; this.options = options; this.callbacks = callbacks;
    this.current = from; this.paused = false; this.fullyQueued = false; this.ids.clear(); this.early.clear();
    await this.serialize(async () => {
      if (revision !== this.revision || this.destroyed) return;
      await disposeNativeAudio();
      await this.initialize();
      await tts.stop();
      this.enqueuing = true;
      try {
        for (let index = from; index < chunks.length; index++) {
          if (revision !== this.revision || this.destroyed) return;
          const response = await tts.speak({ text: chunks[index], voiceId: options.voiceId || null,
            rate: Math.min(4, Math.max(.1, options.rate)), queueMode: index === from ? "flush" : "add" });
          if (revision !== this.revision || this.destroyed) return;
          if (!response.success || !response.utteranceId) throw new Error("原生语音没有确认朗读片段。");
          if (response.warning) callbacks.notice?.("已保存的语音当前不可用，正在使用系统默认语音。");
          this.ids.set(response.utteranceId, index);
          const pending = this.early.get(response.utteranceId) ?? [];
          this.early.delete(response.utteranceId);
          for (const [type, event] of pending) this.event(type, event);
        }
        if (revision === this.revision) this.fullyQueued = true;
      } catch (error) {
        if (revision === this.revision) {
          this.ids.clear(); this.callbacks = null;
          await tts.stop();
          callbacks.error(error);
        }
      } finally { this.enqueuing = false; this.early.clear(); }
    });
  }
  private event(type: SpeechEventType, event: SpeechEvent): void {
    if (!this.callbacks || this.destroyed) return;
    const index = event.id ? this.ids.get(event.id) : undefined;
    if (event.id && index === undefined) {
      if (this.enqueuing && this.early.size < 1024) {
        const pending = this.early.get(event.id) ?? [];
        pending.push([type, event]); this.early.set(event.id, pending);
      }
      return;
    }
    if (type === "speech:start" && index !== undefined) {
      this.current = index; this.paused = false; this.callbacks.start(index);
    } else if (type === "speech:finish" && index === this.chunks.length - 1) {
      const callbacks = this.callbacks; this.callbacks = null; this.ids.clear(); callbacks.finished();
    } else if (type === "speech:pause" || type === "speech:interrupted" || type === "speech:backgroundPause"
      || type === "speech:cancel") {
      if (this.enqueuing) ++this.revision;
      this.paused = true; this.callbacks.paused();
    } else if (type === "speech:resume") {
      this.paused = false; this.callbacks.resumed();
    } else if (type === "speech:error") {
      const callbacks = this.callbacks; this.callbacks = null; this.ids.clear();
      void this.stop().catch(error => console.debug("Native speech cleanup failed:", error)); callbacks.error(event.error || "原生语音服务发生错误。");
    }
  }
  async pause(): Promise<void> {
    if (!this.callbacks || this.paused) return;
    const revision = this.revision, callbacks = this.callbacks;
    const response = await this.serialize(() => tts.pauseSpeaking());
    if (revision !== this.revision || callbacks !== this.callbacks) return;
    if (!response.success) {
      ++this.revision; this.ids.clear(); this.early.clear();
      await this.serialize(() => tts.stop());
    }
    this.paused = true; this.callbacks?.paused();
  }
  async resume(): Promise<void> {
    if (!this.callbacks || !this.paused) return;
    if (!this.fullyQueued) {
      await this.start(this.chunks, this.options, this.callbacks, this.current);
      return;
    }
    const revision = this.revision, pendingCallbacks = this.callbacks;
    const response = await this.serialize(() => tts.resumeSpeaking());
    if (revision !== this.revision || pendingCallbacks !== this.callbacks) return;
    if (response.success) { this.paused = false; this.callbacks?.resumed(); return; }
    const callbacks = this.callbacks;
    await this.start(this.chunks, this.options, callbacks, this.current);
  }
  async stop(): Promise<void> {
    ++this.revision; this.callbacks = null; this.ids.clear(); this.early.clear();
    this.chunks = []; this.paused = false;
    if (this.ready) await this.serialize(() => tts.stop());
  }
  async refresh(): Promise<void> {
    if (this.callbacks && !this.paused && !(await tts.isSpeaking())) {
      if (this.enqueuing) ++this.revision;
      this.paused = true; this.callbacks.paused();
    }
  }
  async dispose(): Promise<void> {
    this.destroyed = true; this.unregisterAudioStart();
    await this.stop(); this.listeners.splice(0).forEach(unlisten => unlisten());
  }
}

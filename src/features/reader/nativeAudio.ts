import { isTauri } from "@tauri-apps/api/core";
import * as audio from "tauri-plugin-native-audio-api";
import type { NativeAudioState } from "tauri-plugin-native-audio-api";

export function usesNativeAudio(): boolean {
  return isTauri() && (/Android|iPhone|iPad|iPod/.test(navigator.userAgent)
    || /Macintosh/.test(navigator.userAgent) && navigator.maxTouchPoints > 1);
}

export type ReaderMediaElement = HTMLMediaElement | NativeReaderAudio;
export interface NativeAudioMetadata {
  key: string;
  title: string;
  artist?: string;
  artworkUrl?: string;
  progressUpdatedAtMs?: number;
}

// The plugin owns one player. Serialize source changes and fence commands from
// disposed readers so a late pause/cleanup cannot affect a newer book.
let active: NativeReaderAudio | null = null;
let commands: Promise<unknown> = Promise.resolve();
let subscription: Promise<void> | null = null;
let unlisten: (() => void | Promise<void>) | undefined;
let beforeAudioStart: (() => Promise<void>) | undefined;
export function registerBeforeNativeAudioStart(callback: () => Promise<void>): () => void {
  beforeAudioStart = callback;
  return () => { if (beforeAudioStart === callback) beforeAudioStart = undefined; };
}
function serialize<T>(operation: () => Promise<T>): Promise<T> {
  const result = commands.catch(() => undefined).then(operation);
  commands = result;
  return result;
}
function listen(): Promise<void> {
  subscription ??= audio.addStateListener(state => active?.accept(state)).then(handle => {
    // 1.0.5's declarations say function; its implementation returns PluginListener.
    const listener = handle as unknown as (() => void) | { unregister: () => Promise<void> };
    unlisten = typeof listener === "function" ? listener : () => listener.unregister();
  }).catch(error => { subscription = null; throw error; });
  return subscription;
}

/** Player adapter for the existing progress/control hooks; no audio plays in the DOM. */
export class NativeReaderAudio extends EventTarget {
  readonly tagName = "AUDIO";
  src = "";
  currentTime = 0;
  duration = Number.NaN;
  paused = true;
  ended = false;
  readyState = 0;
  resumePositionMs: number | null = null;
  error: { code: number; message: string } | null = null;
  private rate = 1;
  private disposed = false;
  private configured = false;
  private sourceRevision = 0;
  private loaded: Promise<void> = Promise.resolve();
  private desiredSeek: number | null = null;

  readonly metadata: NativeAudioMetadata;
  constructor(metadata: NativeAudioMetadata) { super(); this.metadata = metadata; }

  get playbackRate(): number { return this.rate; }
  set playbackRate(value: number) {
    if (!Number.isFinite(value) || value <= 0) return;
    this.rate = value;
    if (this.configured) void this.run(() => audio.setRate(value)).catch(error => this.fail(error));
  }

  load(): void {
    const revision = ++this.sourceRevision, source = this.src;
    this.readyState = 0; this.configured = false; this.error = null; this.ended = false;
    this.loaded = (async () => {
      await beforeAudioStart?.();
      await serialize(async () => {
        if (this.disposed || revision !== this.sourceRevision) return;
        if (!source) throw new Error("音频资源地址为空。");
        await listen();
        await audio.initialize();
        if (this.disposed || revision !== this.sourceRevision) return;
        if (this.metadata.progressUpdatedAtMs !== undefined) {
          const checkpoint = await audio.getProgressCheckpoint();
          if (checkpoint?.id === this.id && checkpoint.updatedAtMs > this.metadata.progressUpdatedAtMs) {
            this.resumePositionMs = Math.max(0, Math.round(checkpoint.currentTime * 1000));
          }
        }
        if (this.disposed || revision !== this.sourceRevision) return;
        active = this;
        const state = await audio.setSource({ src: source, id: this.id,
          title: this.metadata.title, artist: this.metadata.artist, artworkUrl: this.metadata.artworkUrl });
        if (this.disposed || revision !== this.sourceRevision) return;
        this.configured = true;
        this.accept(state);
        this.accept(await audio.setRate(this.rate));
      });
    })();
    void this.loaded.catch(error => { if (revision === this.sourceRevision) this.fail(error); });
  }

  private get id(): number {
    let hash = 2166136261;
    for (const char of this.metadata.key) hash = Math.imul(hash ^ char.charCodeAt(0), 16777619);
    return hash >>> 1;
  }

  private async run(operation: () => Promise<NativeAudioState>): Promise<void> {
    await this.loaded;
    await serialize(async () => {
      if (this.disposed || active !== this || !this.configured) return;
      this.accept(await operation());
    });
  }
  async play(): Promise<void> { await this.run(audio.play); }
  pause(): void { void this.pauseAndWait().catch(error => this.fail(error)); }
  async pauseAndWait(): Promise<void> { await this.run(audio.pause); }
  seek(seconds: number): void {
    if (!Number.isFinite(seconds) || seconds < 0) return;
    this.desiredSeek = seconds;
    void this.run(() => audio.seekTo(seconds)).then(() => {
      if (this.disposed || active !== this || this.desiredSeek !== seconds) return;
      this.desiredSeek = null;
      this.dispatchEvent(new Event("seeked"));
    }).catch(error => { this.desiredSeek = null; this.fail(error); });
  }
  async refresh(): Promise<void> { await this.run(audio.getState); }

  accept(state: NativeAudioState): void {
    if (this.disposed || !this.configured || active !== this) return;
    const wasPaused = this.paused, wasEnded = this.ended, previousRate = this.rate;
    this.currentTime = Math.max(0, state.currentTime);
    this.duration = state.duration > 0 ? state.duration : Number.NaN;
    this.paused = !state.isPlaying;
    this.ended = state.status === "ended";
    this.rate = state.rate;
    if (!this.readyState && Number.isFinite(this.duration)) {
      this.readyState = 1;
      this.dispatchEvent(new Event("loadedmetadata"));
    }
    if (previousRate !== this.rate) this.dispatchEvent(new Event("ratechange"));
    if (wasPaused !== this.paused) this.dispatchEvent(new Event(this.paused ? "pause" : "play"));
    this.dispatchEvent(new Event("timeupdate"));
    if (!wasEnded && this.ended) this.dispatchEvent(new Event("ended"));
    if (state.status === "error") this.fail(state.error || "原生播放器播放失败。");
  }
  private fail(error: unknown): void {
    if (this.disposed) return;
    this.error = { code: 3, message: error instanceof Error ? error.message : String(error) };
    this.paused = true;
    this.dispatchEvent(new Event("error"));
  }
  dispose(): void {
    if (this.disposed) return;
    this.disposed = true; this.sourceRevision++;
    void serialize(async () => {
      if (active !== this) return;
      active = null;
      await audio.dispose();
    }).catch(error => console.error("Native audio cleanup failed:", error));
  }
}

export function seekMedia(media: ReaderMediaElement, seconds: number): void {
  if (media instanceof NativeReaderAudio) media.seek(seconds);
  else media.currentTime = seconds;
}
export async function refreshNativeAudio(): Promise<void> { await active?.refresh(); }
export async function disposeNativeAudio(): Promise<void> {
  active?.dispose();
  await commands.catch(() => undefined);
  await unlisten?.(); unlisten = undefined; subscription = null;
}

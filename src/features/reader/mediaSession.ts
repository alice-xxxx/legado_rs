import { addPluginListener, invoke, isTauri } from "@tauri-apps/api/core";

export const nativeSpeech = isTauri() && (/Android|iPhone|iPad|iPod/.test(navigator.userAgent)
  || /Macintosh/.test(navigator.userAgent) && navigator.maxTouchPoints > 1);

type Actions = Partial<Record<MediaSessionAction, (details: MediaSessionActionDetails) => void | Promise<unknown>>>;
interface MediaControls {
  ready: Promise<void>;
  setPaused(paused: boolean): void;
  sync(media: HTMLMediaElement): void;
  close(): void;
  run(action: MediaSessionAction, details?: MediaSessionActionDetails): void;
}
let active: MediaControls | undefined;
let pending = Promise.resolve();
let nativeListener: Promise<unknown> | undefined;

/** 所有媒体共用一份系统会话；播放始终由网页媒体元素或系统语音负责。 */
export function openMediaSession(metadata: MediaMetadataInit, actions: Actions, onError: (error: unknown) => void): MediaControls {
  active?.close();
  let paused = true;
  let position = -1, duration = 0, rate = 1;
  const browser = nativeSpeech ? undefined : navigator.mediaSession;
  const run = (action: MediaSessionAction, details: MediaSessionActionDetails = { action }) => {
    if (active !== session) return;
    try { void Promise.resolve(actions[action]?.(details)).catch(onError); }
    catch (error) { onError(error); }
  };
  const send = (command: string, data: Record<string, unknown>) => {
    pending = pending.catch(() => {}).then(async () => {
      if (active === session) await invoke(`plugin:media-session|${command}`, data);
    }).catch(error => { if (active === session) onError(error); });
  };
  const session = {
    ready: Promise.resolve(),
    run,
    setPaused(value: boolean) {
      if (active !== session || paused === value) return;
      paused = value;
      if (nativeSpeech) send("update_state", { isPlaying: !paused });
      else if (browser) browser.playbackState = paused ? "paused" : "playing";
    },
    sync(media: HTMLMediaElement) {
      if (active !== session) return;
      const changed = paused !== (media.paused || media.ended);
      session.setPaused(media.paused || media.ended);
      if (!Number.isFinite(media.duration) || media.duration <= 0) return;
      if (!changed && Math.abs(media.currentTime - position) < 1 && duration === media.duration && rate === media.playbackRate) return;
      position = Math.max(0, Math.min(media.currentTime, media.duration));
      duration = media.duration; rate = media.playbackRate;
      if (nativeSpeech) send("update_timeline", { position, duration, playbackSpeed: rate });
      else {
        try { browser?.setPositionState({ position, duration, playbackRate: rate }); }
        catch { /* 部分 WebView 没有时间轴能力。 */ }
      }
    },
    close() {
      if (active !== session) return;
      active = undefined;
      if (nativeSpeech) {
        pending = pending.catch(() => {}).then(async () => {
          if (!active) await invoke("plugin:media-session|clear");
        }).catch(onError);
      } else if (browser) {
        for (const action of Object.keys(actions) as MediaSessionAction[]) {
          try { browser.setActionHandler(action, null); } catch { /* 可选操作。 */ }
        }
        browser.metadata = null;
        browser.playbackState = "none";
        try { browser.setPositionState(); } catch { /* 可选时间轴。 */ }
      }
    },
  };
  active = session;
  if (nativeSpeech) {
    pending = pending.catch(() => {}).then(async () => {
      if (active !== session) return;
      nativeListener ??= addPluginListener<{ action: string; seekPosition?: number }>("media-session", "media_action", event => {
        const action = ({ next: "nexttrack", previous: "previoustrack", seek: "seekto" } as Record<string, MediaSessionAction>)[event.action]
          ?? event.action as MediaSessionAction;
        active?.run(action, { action, seekTime: event.seekPosition });
      }).catch(error => { nativeListener = undefined; throw error; });
      await nativeListener;
    }).catch(error => { if (active === session) onError(error); });
    send("update_state", { title: metadata.title ?? "", artist: metadata.artist ?? "", album: metadata.album ?? "",
      artworkUrl: metadata.artwork?.[0]?.src ?? "", isPlaying: false, duration: 0, position: 0, playbackSpeed: 1,
      canPrev: !!actions.previoustrack, canNext: !!actions.nexttrack, canSeek: !!actions.seekto });
  } else if (browser) {
    browser.metadata = new MediaMetadata(metadata);
    for (const action of Object.keys(actions) as MediaSessionAction[]) {
      try { browser.setActionHandler(action, details => run(action, details)); } catch { /* 可选操作。 */ }
    }
  }
  session.ready = pending;
  return session;
}

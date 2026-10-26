import { addPluginListener, invoke, isTauri } from "@tauri-apps/api/core";

type Actions = Partial<Record<MediaSessionAction, (details: MediaSessionActionDetails) => void | Promise<unknown>>>;
interface MediaControls {
  ready: Promise<void>;
  setPaused(paused: boolean): void;
  sync(media: HTMLMediaElement): void;
  close(): void;
  run(action: MediaSessionAction, details?: MediaSessionActionDetails): void;
}
const mobile = isTauri() && (/Android|iPhone|iPad|iPod/.test(navigator.userAgent)
  || /Macintosh/.test(navigator.userAgent) && navigator.maxTouchPoints > 1);
let active: MediaControls | undefined;
let pending = Promise.resolve();
let listener: Promise<unknown> | undefined;
let nextId = Date.now() * 1024;

/** Playback stays in Vue. This adapter only publishes state and receives system commands. */
export function openMediaSession(metadata: MediaMetadataInit, actions: Actions, onError: (error: unknown) => void): MediaControls {
  active?.close();
  const id = ++nextId;
  const browser = mobile ? undefined : navigator.mediaSession;
  let paused = true, position = 0, duration = 0, rate = 1;
  const run = (action: MediaSessionAction, details: MediaSessionActionDetails = { action }) => {
    if (active !== session) return;
    try { void Promise.resolve(actions[action]?.(details)).catch(onError); }
    catch (error) { onError(error); }
  };
  const send = () => {
    const state = { id, title: metadata.title ?? "", artist: metadata.artist ?? "", album: metadata.album ?? "",
      artwork: metadata.artwork?.[0]?.src ?? "", actions: Object.keys(actions), paused, position, duration, rate };
    pending = pending.catch(() => {}).then(async () => {
      if (active === session) await invoke("plugin:system-media|update", { state });
    }).catch(error => {
      if (active !== session) return;
      onError(error);
    });
  };
  const session: MediaControls = {
    ready: Promise.resolve(), run,
    setPaused(value) {
      if (active !== session || paused === value) return;
      paused = value;
      if (mobile) send();
      else if (browser) browser.playbackState = paused ? "paused" : "playing";
    },
    sync(media) {
      if (active !== session) return;
      const nextPaused = media.paused || media.ended;
      const nextDuration = Number.isFinite(media.duration) ? Math.max(0, media.duration) : 0;
      const time = Number.isFinite(media.currentTime) ? Math.max(0, media.currentTime) : 0;
      const nextPosition = nextDuration ? Math.min(time, nextDuration) : time;
      if (paused === nextPaused && Math.abs(nextPosition - position) < 1 && duration === nextDuration && rate === media.playbackRate) return;
      paused = nextPaused; position = nextPosition; duration = nextDuration; rate = media.playbackRate;
      if (mobile) send();
      else if (browser) {
        browser.playbackState = paused ? "paused" : "playing";
        try {
          if (duration > 0) browser.setPositionState({ position, duration, playbackRate: rate });
          else browser.setPositionState();
        } catch { /* Optional timeline support. */ }
      }
    },
    close() {
      if (active !== session) return;
      active = undefined;
      if (mobile) pending = pending.catch(() => {}).then(() => invoke("plugin:system-media|clear", { id })).then(() => {}, onError);
      else if (browser) {
        for (const action of Object.keys(actions) as MediaSessionAction[]) {
          try { browser.setActionHandler(action, null); } catch { /* Optional action. */ }
        }
        browser.metadata = null; browser.playbackState = "none";
        try { browser.setPositionState(); } catch { /* Optional timeline support. */ }
      }
    },
  };
  active = session;
  if (mobile) {
    pending = pending.catch(() => {}).then(async () => {
      listener ??= addPluginListener<{ id: number; action: MediaSessionAction; value?: number }>("system-media", "action", payload => {
        if (payload.id !== nextId) return;
        active?.run(payload.action, { action: payload.action, seekTime: payload.value, seekOffset: payload.value });
      }).catch(error => { listener = undefined; throw error; });
      await listener;
    });
    send();
  } else if (browser) {
    browser.metadata = new MediaMetadata(metadata);
    for (const action of Object.keys(actions) as MediaSessionAction[]) {
      try { browser.setActionHandler(action, details => run(action, details)); } catch { /* Optional action. */ }
    }
  }
  session.ready = pending;
  return session;
}

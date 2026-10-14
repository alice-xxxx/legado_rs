import assert from "node:assert/strict";
import { registerHooks } from "node:module";

// Node 24 runs these erasable TS adapters directly. No app or native code is built.
const calls = [], speechEvents = new Map();
let listener, state = { status: "idle", currentTime: 0, duration: 120, isPlaying: false, buffering: false, rate: 1 };
let utterance = 0, unregistered = false;
const snapshot = () => ({ ...state });
globalThis.__playbackTest = {
  audio: {
    initialize: async () => snapshot(),
    getProgressCheckpoint: async () => null,
    addStateListener: async fn => { listener = fn; return { unregister: async () => { unregistered = true; } }; },
    setSource: async payload => { calls.push(["source", payload]); state = { ...state, currentTime: 0, isPlaying: false, status: "idle" }; return snapshot(); },
    setRate: async rate => { state.rate = rate; return snapshot(); },
    play: async () => { calls.push(["play"]); state.isPlaying = true; state.status = "playing"; return snapshot(); },
    pause: async () => { state.isPlaying = false; state.status = "idle"; return snapshot(); },
    seekTo: async value => { await new Promise(resolve => setTimeout(resolve, 5)); state.currentTime = value; return snapshot(); },
    getState: async () => snapshot(),
    dispose: async () => { calls.push(["dispose"]); state.isPlaying = false; },
  },
  speech: {
    setBackgroundBehavior: async value => assert.equal(value.continueInBackground, true),
    onSpeechEvent: async (type, fn) => { speechEvents.set(type, fn); return () => speechEvents.delete(type); },
    isInitialized: async () => ({ initialized: true, voiceCount: 1 }),
    getVoices: async () => [{ id: "zh", name: "中文", language: "zh-CN" }],
    speak: async options => {
      const id = String(++utterance); calls.push(["speak", options, id]);
      // Native start can arrive before the speak command's response.
      speechEvents.get("speech:start")?.({ id });
      return { success: true, utteranceId: id };
    },
    stop: async () => { calls.push(["speech-stop"]); },
    pauseSpeaking: async () => ({ success: false, reason: "Android cannot pause TTS" }),
    resumeSpeaking: async () => ({ success: false }),
    isSpeaking: async () => true,
  },
};
registerHooks({
  resolve(specifier, context, next) {
    if (specifier === "tauri-plugin-native-audio-api") return { url: "fixture:audio", shortCircuit: true };
    if (specifier === "tauri-plugin-tts-api") return { url: "fixture:speech", shortCircuit: true };
    if (specifier === "@tauri-apps/api/core") return { url: "fixture:core", shortCircuit: true };
    if (specifier === "./nativeAudio") return next(new URL("./nativeAudio.ts", context.parentURL).href, context);
    return next(specifier, context);
  },
  load(url, context, next) {
    if (!url.startsWith("fixture:")) return next(url, context);
    const key = url.slice(8);
    const source = key === "core" ? "export const isTauri = () => true;"
      : Object.keys(globalThis.__playbackTest[key]).map(name => `export const ${name} = (...args) => globalThis.__playbackTest.${key}.${name}(...args);`).join("\n");
    return { format: "module", source, shortCircuit: true };
  },
});
const { NativeReaderAudio, disposeNativeAudio } = await import("../src/features/reader/nativeAudio.ts");
const { NativeSpeechQueue } = await import("../src/features/reader/nativeSpeech.ts");
const within = promise => Promise.race([promise, new Promise((_, reject) => setTimeout(() => reject(new Error("Playback command deadlock")), 500))]);
const speech = new NativeSpeechQueue();
assert.equal((await speech.voices())[0].id, "zh");
const first = new NativeReaderAudio({ key: "book:0", title: "章节", artist: "作者", artworkUrl: "https://example.org/cover.jpg" });
first.src = "https://example.org/audio.mp3"; first.load(); await within(first.play());
assert.equal(first.paused, false);
assert.equal(calls.find(([type]) => type === "source")[1].artworkUrl, "https://example.org/cover.jpg");
let seeked = false; first.addEventListener("seeked", () => { seeked = true; });
first.seek(42); await new Promise(resolve => setTimeout(resolve, 30));
assert.equal(seeked, true); assert.equal(first.currentTime, 42);
state = { ...state, currentTime: 44, isPlaying: false, status: "idle" }; listener(snapshot());
assert.equal(first.paused, true); assert.equal(first.currentTime, 44, "Interruption state keeps native progress");
console.log("PASS native player metadata, confirmed seeks and interruption state");

const starts = [], errors = []; let finishes = 0, pauses = 0;
const callbacks = { start: index => starts.push(index), paused: () => pauses++, resumed() {}, finished: () => finishes++, error: error => errors.push(error) };
await within(speech.start(["一", "二", "三"], { voiceId: "zh", rate: 1 }, callbacks));
assert.deepEqual(starts, [0, 1, 2], "Early native start events must be correlated after response IDs arrive");
assert.deepEqual(calls.filter(([type]) => type === "speak").map(([, value]) => value.queueMode), ["flush", "add", "add"]);
await within(speech.pause()); assert.equal(pauses, 1);
await within(speech.resume());
assert.equal(calls.filter(([type]) => type === "speak").at(-1)[1].text, "三", "Android continues from the current segment");
const last = calls.filter(([type]) => type === "speak").at(-1)[2];
speechEvents.get("speech:finish")({ id: last }); assert.equal(finishes, 1); assert.deepEqual(errors, []);
console.log("PASS native chapter queue, early events and Android pause/resume fallback");
await speech.start(["暂停前"], { voiceId: "zh", rate: 1 }, callbacks);
let completePause;
globalThis.__playbackTest.speech.pauseSpeaking = () => new Promise(resolve => { completePause = resolve; });
const delayedPause = speech.pause();
await new Promise(resolve => setTimeout(resolve, 0));
const restarted = speech.start(["新的朗读"], { voiceId: "zh", rate: 1 }, callbacks);
completePause({ success: false });
await within(Promise.all([delayedPause, restarted]));
assert.equal(calls.filter(([type]) => type === "speak").at(-1)[1].text, "新的朗读");
assert.equal(pauses, 1, "A late pause response must not cancel or pause newer speech");
globalThis.__playbackTest.speech.pauseSpeaking = async () => ({ success: false });
console.log("PASS stale pause completion does not cancel newer speech");

const second = new NativeReaderAudio({ key: "book:1", title: "下一章" });
second.src = "https://example.org/next.mp3";
const pendingSpeech = speech.start(["旧朗读"], { voiceId: "zh", rate: 1 }, callbacks);
second.load(); await within(Promise.all([pendingSpeech, second.play()]));
const disposedBefore = calls.filter(([type]) => type === "dispose").length;
first.dispose(); await new Promise(resolve => setTimeout(resolve, 10));
assert.equal(calls.filter(([type]) => type === "dispose").length, disposedBefore, "Old source disposal must not stop new playback");
assert.equal(second.paused, false);
const checkpointId = calls.filter(([type]) => type === "source").at(-1)[1].id;
globalThis.__playbackTest.audio.getProgressCheckpoint = async () => ({ id: checkpointId, currentTime: 58, updatedAtMs: 2000 });
const restored = new NativeReaderAudio({ key: "book:1", title: "恢复位置", progressUpdatedAtMs: 1000 });
restored.src = "https://example.org/next.mp3"; restored.load(); await within(restored.play());
assert.equal(restored.resumePositionMs, 58000, "A newer native checkpoint survives WebView suspension/restart");
console.log("PASS native progress checkpoint is considered before reloading the source");
await speech.dispose(); await disposeNativeAudio();
assert.equal(unregistered, true); assert.equal(speechEvents.size, 0);
console.log("PASS switching between speech/audio without deadlocks, stale cleanup and listener disposal");
delete globalThis.__playbackTest;

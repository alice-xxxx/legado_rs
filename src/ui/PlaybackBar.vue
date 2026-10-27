<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import PrototypeIcon from "./PrototypeIcon.vue";

const props = defineProps<{
  title: string;
  subtitle: string;
  status: string;
  icon: "tts" | "audio";
  paused: boolean;
  preparing: boolean;
  disabled: boolean;
  menuOpen: boolean;
  global: boolean;
  error: string;
}>();
defineEmits<{ expand: []; toggle: []; stop: [] }>();

const visible = ref(true);
const hovered = ref(false);
const focused = ref(false);
const statusText = computed(() => props.error || props.status);
let hideTimer: ReturnType<typeof setTimeout> | undefined;
function clearHideTimer(): void {
  clearTimeout(hideTimer);
  hideTimer = undefined;
}
watch([() => props.menuOpen, () => props.preparing, () => props.error, () => props.global, hovered, focused], () => {
  clearHideTimer();
  visible.value = true;
  if (!props.global && !props.menuOpen && !props.preparing && !props.error && !hovered.value && !focused.value) {
    hideTimer = setTimeout(() => { visible.value = false; }, 5000);
  }
}, { immediate: true });
onBeforeUnmount(clearHideTimer);

function onFocusOut(event: FocusEvent): void {
  focused.value = event.relatedTarget instanceof Node && (event.currentTarget as HTMLElement).contains(event.relatedTarget);
}
</script>

<template>
  <div class="playback-bar" :class="{ hidden: !visible, 'above-menu': menuOpen, global }"
    :inert="!visible" :aria-hidden="!visible" role="group" aria-label="播放控制"
    @click.stop @pointerdown.stop @touchstart.stop @touchmove.stop @touchend.stop @wheel.stop
    @pointerenter="hovered = true" @pointerleave="hovered = false"
    @focusin="focused = true" @focusout="onFocusOut">
    <button type="button" class="playback-summary" aria-label="展开播放器" @click="$emit('expand')">
      <span class="playback-symbol" aria-hidden="true"><PrototypeIcon :name="icon" /></span>
      <span class="playback-info">
        <strong>{{ title || '正在播放' }}</strong>
        <span class="playback-caption">
          <span class="playback-status" :class="{ error }" role="status" aria-live="polite">{{ statusText }}</span>
          <span v-if="subtitle" class="playback-chapter">{{ subtitle }}</span>
        </span>
      </span>
    </button>
    <button type="button" class="playback-toggle" :disabled="disabled"
      :aria-label="paused ? '继续播放' : '暂停播放'" :aria-busy="preparing" @click="$emit('toggle')">
      <span v-if="preparing" class="playback-spinner" aria-hidden="true"></span>
      <PrototypeIcon v-else :name="paused ? 'play' : 'pause'" />
    </button>
    <button type="button" class="playback-stop" aria-label="停止播放" @click="$emit('stop')">
      <PrototypeIcon name="close" />
    </button>
  </div>
</template>

<style scoped>
.playback-bar {
  position: absolute;
  z-index: 6;
  inset-inline: max(12px, env(safe-area-inset-left, 0px)) max(12px, env(safe-area-inset-right, 0px));
  bottom: calc(16px + var(--safe-bottom));
  display: flex;
  align-items: center;
  gap: 4px;
  width: auto;
  max-width: 440px;
  margin-inline: auto;
  padding: 6px;
  border: 1px solid var(--reader-chrome-border);
  border-radius: 16px;
  color: var(--reader-text);
  background: var(--reader-chrome-bg);
  box-shadow: var(--app-shadow);
  transition: bottom .25s ease, opacity .2s ease;
}
.playback-bar.above-menu { bottom: calc(70px + var(--safe-bottom)); }
.playback-bar.global { background: var(--app-surface); color: var(--app-text); border-color: var(--app-line); --reader-chrome-text: var(--app-muted); --reader-accent: var(--app-accent); }
.playback-bar { pointer-events: auto; }
.playback-bar.hidden { opacity: 0; pointer-events: none; }
.playback-bar button {
  display: flex;
  align-items: center;
  justify-content: center;
  min-width: 44px;
  min-height: 44px;
  flex: none;
  padding: 0;
  border: 0;
  border-radius: 12px;
  color: inherit;
  background: transparent;
  cursor: pointer;
}
.playback-bar button:focus-visible { outline: 2px solid var(--reader-accent); outline-offset: 2px; }
.playback-bar .playback-summary { flex: 1; min-width: 0; gap: 10px; justify-content: flex-start; padding-inline: 6px; text-align: start; }
.playback-symbol { color: var(--reader-accent); }
.playback-info { display: grid; min-width: 0; gap: 4px; }
.playback-info strong { font-size: var(--ui-font-body); font-weight: 600; }
.playback-info strong, .playback-chapter { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.playback-caption { display: flex; min-width: 0; gap: 8px; color: var(--reader-chrome-text); font-size: var(--ui-font-caption); }
.playback-status { flex: none; }
.playback-status.error { flex: initial; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--app-danger); }
.playback-bar .playback-toggle { color: var(--app-on-accent); background: var(--reader-accent); border-radius: 50%; }
.playback-bar .playback-toggle:disabled { cursor: wait; }
.playback-spinner { width: 20px; height: 20px; border: 2px solid currentColor; border-right-color: transparent; border-radius: 50%; animation: playback-spin .8s linear infinite; }
@keyframes playback-spin { to { transform: rotate(360deg); } }
@media (max-width: 860px) {
  .playback-bar.global { bottom: calc(var(--ui-nav-height) + var(--safe-bottom) + 12px); }
}
@media (prefers-reduced-motion: reduce) {
  .playback-bar { transition: none; }
  .playback-spinner { animation: none; }
}
</style>

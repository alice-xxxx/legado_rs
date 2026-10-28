<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { Button as KButton, Glass as KGlass, Preloader as KPreloader } from "konsta/vue";
import PrototypeIcon from "./PrototypeIcon.vue";

const props = defineProps<{
  title: string;
  subtitle: string;
  status: string;
  icon: "tts" | "audio" | "video";
  paused: boolean;
  preparing: boolean;
  disabled: boolean;
  menuOpen: boolean;
  global: boolean;
  navigation?: boolean;
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
  <KGlass class="playback-bar rounded-2xl material:bg-md-light-surface-3 material:dark:bg-md-dark-surface-3" :class="{ 'is-hidden': !visible, 'above-menu': menuOpen, 'with-navigation': navigation, global }"
    :inert="!visible" :aria-hidden="!visible" role="group" aria-label="播放控制"
    @click.stop @pointerdown.stop @touchstart.stop @touchmove.stop @touchend.stop @wheel.stop
    @pointerenter="hovered = true" @pointerleave="hovered = false"
    @focusin="focused = true" @focusout="onFocusOut">
    <KButton rounded inline clear type="button" :colors="{ textIos: 'text-current', textMaterial: 'text-current' }"
      class="playback-summary h-auto min-h-11 justify-start gap-2 text-start" aria-label="展开播放器" @click="$emit('expand')">
      <span class="playback-symbol" aria-hidden="true"><PrototypeIcon :name="icon" /></span>
      <span class="playback-info">
        <strong>{{ title || '正在播放' }}</strong>
        <span class="playback-caption">
          <span class="playback-status" :class="{ error }" role="status" aria-live="polite">{{ statusText }}</span>
          <span v-if="subtitle" class="playback-chapter">{{ subtitle }}</span>
        </span>
      </span>
    </KButton>
    <KButton inline rounded type="button" class="playback-toggle size-11 shrink-0 p-0" :disabled="disabled"
      :aria-label="paused ? '继续播放' : '暂停播放'" :aria-busy="preparing" @click="$emit('toggle')">
      <KPreloader v-if="preparing" class="size-5" aria-hidden="true" />
      <PrototypeIcon v-else :name="paused ? 'play' : 'pause'" />
    </KButton>
    <KButton inline clear rounded type="button" class="playback-stop size-11 shrink-0 p-0" aria-label="停止播放" @click="$emit('stop')">
      <PrototypeIcon name="close" />
    </KButton>
  </KGlass>
</template>

<style scoped>
@layer components {
.playback-bar {
  --reader-accent: var(--app-accent);
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
  color: var(--app-text);
  transition: bottom .25s ease, opacity .2s ease;
}
.playback-bar.above-menu { bottom: calc(var(--ui-nav-height) + var(--safe-bottom) + 12px); }
.playback-bar { pointer-events: auto; }
.playback-bar.is-hidden { opacity: 0; pointer-events: none; }
.playback-bar button:focus-visible { outline: 2px solid var(--reader-accent); outline-offset: 2px; }
.playback-bar .playback-summary { flex: 1; min-width: 0; gap: 10px; justify-content: flex-start; padding-inline: 6px; text-align: start; }
.playback-symbol { color: var(--reader-accent); }
.playback-info { display: grid; min-width: 0; gap: 4px; }
.playback-info strong { font-size: var(--ui-font-body); font-weight: 600; }
.playback-info strong, .playback-chapter { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.playback-caption { display: flex; min-width: 0; gap: 8px; color: var(--app-muted); font-size: var(--ui-font-caption); }
.playback-status { flex: none; }
.playback-status.error { flex: initial; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--app-danger); }
@media (max-width: 860px) {
  .playback-bar.global.with-navigation { bottom: calc(var(--ui-nav-height) + var(--safe-bottom) + 12px); }
}
@media (prefers-reduced-motion: reduce) {
  .playback-bar { transition: none; }
}
}
</style>

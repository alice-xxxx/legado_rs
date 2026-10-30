<script setup lang="ts">
import { Popover as KPopover } from "konsta/vue";
import { nextTick, onBeforeUnmount, onMounted, ref, useId, watch, type CSSProperties } from "vue";

defineOptions({ inheritAttrs: false });
const props = defineProps<{ target?: HTMLElement | string; opened?: boolean; backdrop?: boolean }>();
const panel = ref<HTMLElement>();
const ownerId = ref("");
const generatedId = `dropdown-${useId()}`;
const position = ref<CSSProperties>({ visibility: "hidden" });
function place(ensureSpace = false): void {
  const target = typeof props.target === "string" ? document.querySelector<HTMLElement>(props.target) : props.target;
  const element = panel.value;
  if (!props.opened || !target || !element) return;
  const viewportHeight = window.visualViewport?.height ?? window.innerHeight;
  if (ensureSpace && viewportHeight - target.getBoundingClientRect().bottom < 120) target.scrollIntoView({ block: "center", behavior: "instant" });
  ownerId.value = target.id ||= generatedId;
  const bounds = target.getBoundingClientRect();
  const width = Math.min(element.offsetWidth, document.documentElement.clientWidth - 32);
  const top = bounds.bottom + 4;
  position.value = {
    left: `${Math.max(16, Math.min(bounds.right - width, document.documentElement.clientWidth - width - 16))}px`,
    top: `${top}px`, maxWidth: "calc(100vw - 32px)",
    "--dropdown-height": `${Math.max(0, viewportHeight - top - 16)}px`,
  };
}
function reposition(event: Event): void {
  if (!(event.target instanceof Node && panel.value?.contains(event.target))) place();
}
onMounted(() => {
  place(true);
  window.addEventListener("resize", reposition);
  window.visualViewport?.addEventListener("resize", reposition);
  document.addEventListener("scroll", reposition, true);
});
onBeforeUnmount(() => {
  window.removeEventListener("resize", reposition);
  window.visualViewport?.removeEventListener("resize", reposition);
  document.removeEventListener("scroll", reposition, true);
});
watch(() => [props.opened, props.target], async () => { await nextTick(); place(true); });
</script>

<template>
  <Teleport to="body">
    <div ref="panel" v-bind="$attrs" :style="position" :data-popover-owner="ownerId" class="dropdown-popover fixed z-[190]">
      <KPopover :opened="opened" :backdrop="backdrop ?? false" class="static! w-full! transform-none! origin-top-right!">
        <slot />
      </KPopover>
    </div>
  </Teleport>
</template>

<style scoped>
@layer components {
.dropdown-popover { width: 240px; }
.dropdown-popover :deep(.k-popover > .k-glass) { max-height: var(--dropdown-height); overflow-y: auto !important; overscroll-behavior: contain; }
}
</style>

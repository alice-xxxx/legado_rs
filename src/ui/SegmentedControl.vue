<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";
import { Segmented } from "konsta/vue";

const ready = ref(false);
let frame = 0;
onMounted(() => {
  // Paint Konsta's initial highlight position before enabling transitions.
  frame = requestAnimationFrame(() => {
    frame = requestAnimationFrame(() => { ready.value = true; });
  });
});
onBeforeUnmount(() => cancelAnimationFrame(frame));
</script>

<template>
  <Segmented :class="{ 'segmented-initializing': !ready }"><slot /></Segmented>
</template>

<style scoped>
@layer components {
.segmented-initializing :deep(> span:last-child) { transition: none; }
}
</style>

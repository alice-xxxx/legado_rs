<script setup lang="ts">
import SubpageHeader from "../../ui/SubpageHeader.vue";
import { routeNames } from "../../router";
import { returnToRoute } from "../../app/routeReturn";

defineProps<{ title: string }>();

function returnToAdvancedSettings(): void {
  returnToRoute(routeNames.settingsOther);
}
</script>

<template>
  <SubpageHeader :title="title" back-label="返回高级设置" @back="returnToAdvancedSettings" />
</template>

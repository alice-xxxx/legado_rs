<script setup lang="ts">
import { List as KList } from "konsta/vue";
import SettingsActionRow from "../../ui/SettingsActionRow.vue";
import SubpageHeader from "../../ui/SubpageHeader.vue";
import router, { routeNames } from "../../router";
import { returnToRoute } from "../../app/routeReturn";

const groups = [
  { title: "解析与规则", items: [
    { title: "替换与净化", icon: "rule", route: routeNames.settingsRules },
    { title: "TXT 目录识别", icon: "text", route: routeNames.settingsTxtToc },
    { title: "发现页配置", icon: "tune", route: routeNames.settingsHomeConfig },
  ] },
  { title: "网络与缓存", items: [
    { title: "正文缓存", icon: "download", route: routeNames.settingsCache },
    { title: "网络请求", icon: "network", route: routeNames.settingsNetwork },
  ] },
  { title: "朗读", items: [
    { title: "TTS 引擎", icon: "tts", route: routeNames.settingsTts },
  ] },
] as const;
</script>

<template>
  <section class="other-settings-page">
    <SubpageHeader title="高级设置" back-label="返回我的" @back="returnToRoute(routeNames.settings)" />
    <div class="ui-page-content">
      <section v-for="group in groups" :key="group.title" class="ui-settings-group">
        <h2>{{ group.title }}</h2>
        <KList strong inset dividers class="mx-0! my-0!">
          <SettingsActionRow v-for="item in group.items" :key="item.route"
            :title="item.title" :icon="item.icon" @click="router.push({ name: item.route })" />
        </KList>
      </section>
    </div>
  </section>
</template>

<style scoped>
@layer components {
.other-settings-page { min-height: 100dvh; color: var(--app-text); }
}
</style>

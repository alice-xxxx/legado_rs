<script setup lang="ts">
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import BackButton from "../../ui/BackButton.vue";
import router, { routeNames, settingsRouteNames, type SettingsSection } from "../../router";
import { returnToRoute } from "../../app/routeReturn";

function openSection(section: SettingsSection): void {
  void router.push({ name: settingsRouteNames[section] });
}
function openDetail(detail: "cache" | "network" | "tts"): void {
  const name = detail === "cache" ? routeNames.settingsCache
    : detail === "network" ? routeNames.settingsNetwork : routeNames.settingsTts;
  void router.push({ name });
}
function openHomeConfig(): void {
  void router.push({ name: routeNames.settingsHomeConfig });
}
</script>

<template>
  <section class="other-settings-page">
    <header class="other-home-topbar safe-area-top">
        <BackButton
          class="other-home-back"
          label="返回我的"
          @click="returnToRoute(routeNames.settings)"
        />
        <div><strong>高级设置</strong></div>
        <span></span>
      </header>
      <div class="ui-page-content settings-menu">
      <div class="settings-menu-section">
        <small class="settings-menu-label">解析与规则</small>
        <button type="button" class="settings-menu-row" @click="openSection('rules')">
          <span class="settings-menu-icon"><PrototypeIcon name="rule"/></span>
          <span class="settings-menu-copy"><strong>替换与净化</strong></span><PrototypeIcon name="right"/>
        </button>
        <button type="button" class="settings-menu-row" @click="openSection('txt-toc')">
          <span class="settings-menu-icon"><PrototypeIcon name="text"/></span>
          <span class="settings-menu-copy"><strong>TXT 目录识别</strong></span><PrototypeIcon name="right"/>
        </button>
        <button type="button" class="settings-menu-row" @click="openHomeConfig()">
          <span class="settings-menu-icon"><PrototypeIcon name="tune"/></span>
          <span class="settings-menu-copy"><strong>发现页配置</strong></span><PrototypeIcon name="right"/>
        </button>
      </div>
      <div class="settings-menu-section">
        <small class="settings-menu-label">网络与缓存</small>
        <button type="button" class="settings-menu-row" @click="openDetail('cache')">
          <span class="settings-menu-icon"><PrototypeIcon name="download"/></span>
          <span class="settings-menu-copy"><strong>正文缓存</strong></span><PrototypeIcon name="right"/>
        </button>
        <button type="button" class="settings-menu-row" @click="openDetail('network')">
          <span class="settings-menu-icon"><PrototypeIcon name="network"/></span>
          <span class="settings-menu-copy"><strong>网络请求</strong></span><PrototypeIcon name="right"/>
        </button>
      </div>
      <div class="settings-menu-section">
        <small class="settings-menu-label">朗读</small>
        <button type="button" class="settings-menu-row" @click="openDetail('tts')">
          <span class="settings-menu-icon"><PrototypeIcon name="tts"/></span>
          <span class="settings-menu-copy"><strong>TTS 引擎</strong></span><PrototypeIcon name="right"/>
        </button>
      </div>
      </div>
  </section>
</template>
<style scoped>
.other-settings-page {
  width: 100%;
  min-width: 0;
  min-height: 100dvh;
  color: var(--app-text);
}

.other-home-topbar {
  position: sticky;
  top: 0;
  z-index: 35;
  box-sizing: border-box;
  display: grid;
  grid-template-columns: 44px minmax(0, 1fr) 42px;
  align-items: center;
  gap: 10px;
  min-height: var(--ui-header-height);
  padding-inline: var(--ui-header-gutter);
  border-bottom: 1px solid var(--app-line);
  background: var(--app-header);
  backdrop-filter: blur(18px);
}

.other-home-topbar strong {
  color: var(--app-text);
  font-size: var(--ui-font-heading);
  font-weight: 750;
}

.other-home-topbar {
    padding-inline: var(--ui-header-gutter);
  }
</style>

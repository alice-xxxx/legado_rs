<script setup lang="ts">
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import BackButton from "../../ui/BackButton.vue";
import router, { routeNames, settingsRouteNames, type SettingsSection } from "../../router";

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
    <header class="other-home-topbar">
        <BackButton
          class="other-home-back"
          label="返回我的"
          @click="openSection('home')"
        />
        <div><strong>高级设置</strong></div>
        <span></span>
      </header>
      <div class="other-home-content">
      <div class="settings-link-group">
        <small class="settings-group-label">解析与规则</small>
        <button type="button" class="settings-link-row" @click="openSection('rules')">
          <span class="settings-link-icon"><PrototypeIcon name="rule"/></span>
          <span><strong>替换与净化</strong></span><PrototypeIcon name="right"/>
        </button>
        <button type="button" class="settings-link-row" @click="openSection('txt-toc')">
          <span class="settings-link-icon"><PrototypeIcon name="text"/></span>
          <span><strong>TXT 目录识别</strong></span><PrototypeIcon name="right"/>
        </button>
        <button type="button" class="settings-link-row" @click="openHomeConfig()">
          <span class="settings-link-icon"><PrototypeIcon name="tune"/></span>
          <span><strong>发现页配置</strong></span><PrototypeIcon name="right"/>
        </button>
      </div>
      <div class="settings-link-group">
        <small class="settings-group-label">网络与缓存</small>
        <button type="button" class="settings-link-row" @click="openDetail('cache')">
          <span class="settings-link-icon"><PrototypeIcon name="download"/></span>
          <span><strong>正文缓存</strong></span><PrototypeIcon name="right"/>
        </button>
        <button type="button" class="settings-link-row" @click="openDetail('network')">
          <span class="settings-link-icon"><PrototypeIcon name="network"/></span>
          <span><strong>网络请求</strong></span><PrototypeIcon name="right"/>
        </button>
      </div>
      <div class="settings-link-group">
        <small class="settings-group-label">朗读</small>
        <button type="button" class="settings-link-row" @click="openDetail('tts')">
          <span class="settings-link-icon"><PrototypeIcon name="tts"/></span>
          <span><strong>TTS 引擎</strong></span><PrototypeIcon name="right"/>
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
  height: 64px;
  padding: 0 max(18px, calc((100% - 1180px) / 2));
  border-bottom: 1px solid var(--app-line);
  background: #f6f7f9ed;
  backdrop-filter: blur(18px);
}

.other-home-topbar strong {
  color: var(--app-text);
  font-size: 14px;
  font-weight: 750;
}

.other-home-content {
  box-sizing: border-box;
  width: min(var(--app-content-width), calc(100% - 40px));
  margin: 0 auto;
  padding: 24px 0 54px;
}

.settings-link-group {
  margin-bottom: 24px;
}

.settings-group-label {
  display: block;
  margin: 0 4px 6px;
  color: var(--app-muted);
  font-size: 10px;
  font-weight: 650;
}

.settings-link-row {
  display: grid;
  grid-template-columns: 40px minmax(0, 1fr) 20px;
  align-items: center;
  gap: 12px;
  width: 100%;
  min-height: 66px;
  padding: 11px 8px;
  border: 0;
  border-bottom: 1px solid var(--app-line);
  background: transparent;
  text-align: left;
  cursor: pointer;
}

.settings-link-row:hover {
  background: var(--app-accent-soft);
}

.settings-link-row > span:nth-child(2) {
  min-width: 0;
}

.settings-link-row strong {
  color: var(--app-text);
  font-size: 12px;
  font-weight: 650;
}

.settings-link-icon {
  display: grid;
  place-items: center;
  width: 34px;
  height: 34px;
  border-radius: 10px;
  color: var(--app-icon-ink);
  background: var(--app-icon-background);
}

.settings-link-icon :deep(svg) {
  width: 19px;
  height: 19px;
}

.settings-link-row > .prototype-icon {
  width: 18px;
  height: 18px;
  color: var(--app-subtle);
}

.other-home-topbar {
    padding: 0 18px;
  }
</style>

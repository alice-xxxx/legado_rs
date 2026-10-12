<script setup lang="ts">
import { computed } from "vue";
import { settings } from "./settingsStore";
import { backupBusy } from "./backupState";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import SettingsDetailHeader from "./SettingsDetailHeader.vue";

const operationBlocked = computed(() => backupBusy.value !== null || shelfBatchRecoveryRequired.value);
const controlsDisabled = operationBlocked;

function changeTimeout(seconds: number): void {
  if (controlsDisabled.value) return;
  settings.value.sourceHttpTimeoutSeconds = seconds;
}

</script>

<template>
  <section class="settings-detail-page">
    <SettingsDetailHeader title="网络请求" />
    <div class="settings-detail-content">
      <section class="settings-detail-panel">
        <header class="settings-detail-section-header"><strong>来源网络请求</strong></header>
        <div class="setting-control">
          <div>
            <label for="source-http-timeout">最长时限</label>
            <small>连接或读取超时后结束请求</small>
          </div>
          <select
            id="source-http-timeout"
            :value="settings.sourceHttpTimeoutSeconds"
            :disabled="controlsDisabled"
            @change="changeTimeout(Number(($event.target as HTMLSelectElement).value))"
          >
            <option :value="15">15 秒</option>
            <option :value="30">30 秒</option>
            <option :value="60">60 秒</option>
            <option :value="120">120 秒</option>
          </select>
        </div>
      </section>
    </div>
  </section>
</template>

<style scoped>
.settings-detail-page {
  width: min(820px, 100%);
  min-height: 100dvh;
  margin: 0 auto;
  padding: 10px 0 50px;
  color: var(--app-text);
}

.settings-detail-content { display: grid; gap: 18px; }
.settings-detail-panel { padding: 0; border: 0; border-radius: 0; background: transparent; }
.settings-detail-section-header {
  display: grid;
  gap: 4px;
  margin: 0 0 9px;
  padding: 0 2px 12px;
  border-bottom: 1px solid var(--app-line);
}
.settings-detail-section-header strong { color: var(--app-text); font-size: var(--ui-font-caption); }
.setting-control {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  min-height: 68px;
  padding: 8px 4px;
  border-bottom: 1px solid var(--app-line);
}
.setting-control > div { display: grid; gap: 4px; min-width: 0; }
.setting-control label { color: var(--app-text); font-size: var(--ui-font-body); font-weight: 650; }
.setting-control small { color: var(--app-subtle); font-size: var(--ui-font-caption); line-height: 1.5; }
.setting-control select {
  min-width: 105px;
  min-height: 36px;
  padding: 5px;
  border: 1px solid var(--app-line);
  border-radius: 8px;
  background: var(--app-surface);
}
@media (max-width: 640px) {
  .settings-detail-content { gap: 22px; }
  .setting-control { min-height: 65px; align-items: center; }
  .setting-control > div { flex: 1; max-width: 60%; }
}
</style>

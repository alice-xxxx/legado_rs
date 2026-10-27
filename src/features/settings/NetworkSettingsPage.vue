<script setup lang="ts">
import { ListInput as KListInput } from "konsta/vue";
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
        <div class="setting-control">
          <div>
            <label for="source-http-timeout">请求超时</label>
            <small>连接或读取超时后结束请求</small>
          </div>
          <KListInput component="div" input-id="source-http-timeout" :value="settings.sourceHttpTimeoutSeconds" :disabled="controlsDisabled" type="select" @change="changeTimeout(Number(($event.target as HTMLSelectElement).value))">
            <option :value="15">15 秒</option>
            <option :value="30">30 秒</option>
            <option :value="60">60 秒</option>
            <option :value="120">120 秒</option>
          </KListInput>
        </div>
    </div>
  </section>
</template>

<style scoped>
@layer components {
.settings-detail-page {
  width: 100%;
  min-width: 0;
  min-height: 100dvh;
  color: var(--app-text);
}

.settings-detail-content {
  width: min(var(--app-content-width), 100%);
  margin: 0 auto;
  padding: var(--ui-space-6) var(--ui-page-gutter) var(--ui-space-8);
}
.setting-control {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ui-space-4);
  min-height: 64px;
  padding: 0 0 var(--ui-space-3);
  border-bottom: 1px solid var(--app-line);
}
.setting-control > div { display: grid; gap: var(--ui-space-1); min-width: 0; }
.setting-control label { color: var(--app-text); font-size: var(--ui-font-body); font-weight: 500; }
.setting-control small { color: var(--app-muted); font-size: var(--ui-font-caption); line-height: 1.5; }
}
</style>

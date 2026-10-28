<script setup lang="ts">
import { List as KList, ListItem as KListItem } from "konsta/vue";
import SelectMenu from "../../ui/SelectMenu.vue";
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
    <div class="settings-detail-content ui-page-content">
      <KList strong inset class="mx-0! my-0!">
        <KListItem title-font-size-ios="text-[length:var(--ui-font-body)]" title-font-size-material="text-[length:var(--ui-font-body)]" title="请求超时" subtitle="连接或读取超时后结束请求">
          <template #after>
            <SelectMenu id="source-http-timeout" label="请求超时" :model-value="settings.sourceHttpTimeoutSeconds" :disabled="controlsDisabled"
              :options="[15, 30, 60, 120].map(value => ({ value, label: value + ' 秒' }))" @update:model-value="changeTimeout" />
          </template>
        </KListItem>
      </KList>
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

}
</style>

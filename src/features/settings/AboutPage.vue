<script setup lang="ts">
import { List as KList, ListItem as KListItem, Toggle as KToggle, Button as KButton } from "konsta/vue";
import { openUrl } from "@tauri-apps/plugin-opener";
import license from "../../../LICENSE?raw";
import thirdPartyNotices from "../../../THIRD_PARTY_NOTICES.txt?raw";
import privacy from "../../../PRIVACY.txt?raw";
import { settings } from "./settingsStore";
import { errorText, notify } from "../../app/notifications";
import SettingsActionRow from "../../ui/SettingsActionRow.vue";
import SubpageHeader from "../../ui/SubpageHeader.vue";
import { routeNames } from "../../router";
import { returnToRoute } from "../../app/routeReturn";
import { appUpdate, checkAppUpdate, currentPlatform, currentVersion, updateBlocked, updateDialogOpen, updateError, updatePhase } from "./appUpdates";

async function openLink(url: string): Promise<void> {
  try { await openUrl(url); }
  catch (error) { notify(`打开链接失败：${errorText(error)}`, "error"); }
}

function openFeedback(): void {
  const body = `### 环境信息\n版本：${currentVersion.value || '未知'}\n平台：${currentPlatform.value || '未知'}\n\n### 问题描述\n\n### 复现步骤\n\n### 预期结果\n\n请勿附上密码、Cookie、访问令牌或完整备份文件。`;
  void openLink(`https://github.com/alice-xxxx/legado_rs/issues/new?${new URLSearchParams({ body })}`);
}
</script>

<template>
  <section class="min-h-dvh text-[var(--app-text)]">
    <SubpageHeader title="关于" back-label="返回我的" @back="returnToRoute(routeNames.settings)" />
    <div class="settings-detail-content ui-page-content">
      <h2 class="text-[length:var(--ui-font-heading)] font-semibold mb-4">Foliver</h2>
      <KList strong inset dividers class="mx-0! my-0!">
        <KListItem title="当前版本" :after="currentVersion ? `v${currentVersion}` : '开发预览'"
          title-font-size-ios="text-[length:var(--ui-font-body)]" title-font-size-material="text-[length:var(--ui-font-body)]" />
        <KListItem title="运行平台" :after="currentPlatform || '未知'"
          title-font-size-ios="text-[length:var(--ui-font-body)]" title-font-size-material="text-[length:var(--ui-font-body)]" />
        <SettingsActionRow title="GitHub 项目" subtitle="alice-xxxx/legado_rs" icon="link" @click="openLink('https://github.com/alice-xxxx/legado_rs')" />
        <SettingsActionRow title="问题反馈" subtitle="前往 GitHub Issues，附带版本与平台信息" icon="edit" @click="openFeedback" />
        <KListItem title="启动时检查更新" subtitle="仅在启动时检查正式发布版本"
          title-font-size-ios="text-[length:var(--ui-font-body)]" title-font-size-material="text-[length:var(--ui-font-body)]">
          <template #after><KToggle :checked="settings.autoCheckUpdates !== false" :disabled="updateBlocked" @change="settings.autoCheckUpdates = ($event.target as HTMLInputElement).checked"><span class="sr-only">启动时检查更新</span></KToggle></template>
        </KListItem>
      </KList>
      <div class="flex flex-wrap gap-3 mt-4">
        <KButton rounded inline :disabled="updateBlocked || updatePhase === 'restart'" @click="checkAppUpdate()">{{ updatePhase === 'checking' ? '检查中…' : '检查更新' }}</KButton>
        <KButton v-if="appUpdate" rounded inline outline @click="updateDialogOpen = true">{{ updatePhase === 'restart' ? '重启完成更新' : `查看 v${appUpdate.version}` }}</KButton>
      </div>
      <p v-if="updateError" class="mt-4 text-[var(--app-danger)]" role="alert">{{ updateError }}</p>
      <details class="mt-6">
        <summary class="cursor-pointer">开源许可 · GNU GPL v3</summary>
        <p class="my-3">Copyright © 2026 Foliver Contributors。允许依照 GNU GPL v3 使用、修改和再分发；本软件不提供任何担保。第三方组件遵循各自的许可。</p>
        <pre class="max-h-[50dvh] overflow-auto whitespace-pre-wrap break-words text-sm">{{ license }}</pre>
      </details>
      <details class="mt-4">
        <summary class="cursor-pointer">第三方许可与致谢</summary>
        <pre class="mt-3 max-h-[50dvh] overflow-auto whitespace-pre-wrap break-words text-sm">{{ thirdPartyNotices }}</pre>
      </details>
      <details class="mt-4">
        <summary class="cursor-pointer">隐私与数据说明</summary>
        <pre class="mt-3 whitespace-pre-wrap break-words font-[inherit]">{{ privacy }}</pre>
      </details>
    </div>
  </section>
</template>

<script setup lang="ts">
// 页面拥有备份 UI 与动作；阅读器只通过窄 session 暴露备份所需能力。
import { computed, ref, watch } from "vue";
import { backupBusy } from "./backupState";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { useBackupActions } from "./useBackupActions";
import { settings as sharedSettings } from "./settingsStore";
import { routeNames } from "../../router";
import { returnToRoute } from "../../app/routeReturn";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import BackButton from "../../ui/BackButton.vue";

const {
  webDavBackupConfig: webDavConfig,
  webDavBackupError: webDavError,
  createBackup,
  restoreBackup,
  saveWebDavConfig,
  clearWebDavPassword,
  uploadWebDavBackupManually,
  restoreWebDavBackupManually,
} = useBackupActions();
const busy = backupBusy;
const settings = sharedSettings;
const locked = computed(() => shelfBatchRecoveryRequired.value);

function returnToSettings(): void {
  returnToRoute(routeNames.settings);
}

const webDavUrl = ref("");
const webDavUsername = ref("");
const webDavPassword = ref("");
const autoIntervalHours = computed(() => webDavConfig.value?.autoIntervalHours ?? null);
const configRefreshConflict = ref(false);

function resetWebDavDraft(): void {
  const config = webDavConfig.value;
  webDavUrl.value = config?.url ?? "";
  webDavUsername.value = config?.username ?? "";
  webDavPassword.value = "";
  configRefreshConflict.value = false;
}
watch(() => webDavConfig.value, (config, previous) => {
  if (!config) return;
  const dirty = previous && (
    webDavUrl.value.trim() !== previous.url ||
    webDavUsername.value.trim() !== previous.username ||
    webDavPassword.value.length > 0
  );
  const matchesDraft = webDavUrl.value.trim() === config.url
    && webDavUsername.value.trim() === config.username && !webDavPassword.value;
  if (dirty && !matchesDraft) {
    configRefreshConflict.value = true;
    return;
  }
  resetWebDavDraft();
}, { immediate: true });

const webDavConfigMatches = computed(() => Boolean(webDavConfig.value?.url)
  && webDavUrl.value.trim() === webDavConfig.value?.url
  && webDavUsername.value.trim() === webDavConfig.value?.username
  && webDavPassword.value.length === 0);
const sameWebDavIdentity = computed(() => Boolean(webDavConfig.value)
  && webDavUrl.value.trim() === webDavConfig.value?.url
  && webDavUsername.value.trim() === webDavConfig.value?.username);
const canEditAutoSchedule = computed(() => Boolean(webDavConfig.value?.url)
  && webDavConfigMatches.value
  && !locked.value
  && !busy.value);

const lastBackup = computed(() => settings.value.lastBackupAtMs
  ? new Date(settings.value.lastBackupAtMs).toLocaleString() : "尚未备份");

function changeAutoBackupInterval(event: Event): void {
  if (!canEditAutoSchedule.value || !settings.value.webdavBackup) return;
  const raw = (event.target as HTMLSelectElement).value;
  settings.value.webdavBackup.autoIntervalHours = raw ? Number(raw) : null;
}

function saveWebDavDraft(): void {
  if (saveWebDavConfig({ url: webDavUrl.value.trim(), username: webDavUsername.value.trim(), password: webDavPassword.value })) {
    webDavPassword.value = "";
    webDavUrl.value = webDavConfig.value?.url ?? "";
  }
}
</script>

<template>
  <section class="backup-settings-page my-settings-subpage">
    <header class="backup-topbar safe-area-top">
      <BackButton label="返回我的" @click="returnToSettings" />
      <div class="backup-page-title"><strong>数据与同步</strong></div>
      <span></span>
    </header>
    <div class="ui-page-content backup-content">
    <div class="backup-summary">
      <span>最近备份</span><span>{{ lastBackup }}</span>
    </div>

    <section class="backup-local-section">
      <h2>本地备份</h2>
      <button type="button" class="settings-menu-row" :disabled="!!busy || locked" @click="createBackup">
        <PrototypeIcon name="export"/>
        <span class="settings-menu-copy"><strong>{{ busy === 'create' ? '正在导出…' : '导出备份文件' }}</strong></span><PrototypeIcon name="right"/>
      </button>
      <button type="button" class="settings-menu-row" :disabled="!!busy || locked" @click="restoreBackup">
        <PrototypeIcon name="import"/>
        <span class="settings-menu-copy"><strong>{{ busy === 'restore' ? '正在恢复…' : '从备份恢复' }}</strong><small>恢复会替换当前本地数据</small></span><PrototypeIcon name="right"/>
      </button>
    </section>

    <section class="webdav-settings">
      <h2 id="webdav-backup-heading">WebDAV 备份</h2>

      <form class="webdav-config-form" @submit.prevent="saveWebDavDraft">
        <label class="webdav-setting-row">
          <span class="backup-row-copy"><strong>目录地址</strong></span>
          <input v-model="webDavUrl" type="url" inputmode="url" autocomplete="url" maxlength="4096" placeholder="https://example.com/dav/" :disabled="!!busy || locked" required />
        </label>
        <label class="webdav-setting-row">
          <span class="backup-row-copy"><strong>用户名</strong></span>
          <input v-model="webDavUsername" autocomplete="username" maxlength="512" placeholder="输入用户名" :disabled="!!busy || locked" />
        </label>
        <label class="webdav-setting-row">
          <span class="backup-row-copy"><strong>密码</strong></span>
          <input v-model="webDavPassword" type="password" autocomplete="new-password" maxlength="4096" :placeholder="webDavConfig?.password ? sameWebDavIdentity ? '已保存；留空保持不变' : '更换目录或用户名时需重新输入密码' : '输入 WebDAV 密码'" :disabled="!!busy || locked" />
        </label>
        <div v-if="configRefreshConflict" class="webdav-draft-warning">
          已保存的 WebDAV 配置发生变化。当前输入仍保留，若要放弃草稿请点击重新读取表单。
          <button type="button" :disabled="!!busy || locked" @click="resetWebDavDraft">重新读取表单</button>
        </div>
        <div class="webdav-config-actions">
          <button type="submit" class="button secondary small" :disabled="!!busy || locked || !webDavUrl.trim()">保存设置</button>
          <button v-if="webDavConfig?.password" type="button" class="text-button" :disabled="!!busy || locked || !webDavConfigMatches" @click="clearWebDavPassword">清除已保存密码</button>
        </div>
      </form>

      <p v-if="webDavConfig?.url" class="webdav-config-state">
        {{ webDavConfigMatches ? '连接设置已保存' : '有未保存的修改' }}
        <span v-if="webDavConfigMatches"> · {{ webDavConfig.password ? '密码已保存' : '未保存密码' }}</span>
      </p>
      <details class="webdav-help">
        <summary>连接说明</summary>
        <p>远程目录须已存在。密码不会回填；目录和用户名不变时，密码留空会保留已存密码，更换目录或用户名时需重新输入密码。</p>
      </details>

      <div class="webdav-setting-row webdav-auto-setting">
        <label class="backup-row-copy" for="webdav-auto-interval"><strong>自动备份</strong><small>应用运行期间备份已保存的数据</small></label>
        <select id="webdav-auto-interval" :value="autoIntervalHours ?? ''" :disabled="!canEditAutoSchedule" @change="changeAutoBackupInterval">
          <option value="">关闭</option><option value="6">每 6 小时</option><option value="12">每 12 小时</option><option value="24">每天</option><option value="72">每 3 天</option>
        </select>
      </div>

      <div class="webdav-actions">
        <button type="button" class="button secondary small" :disabled="!!busy || locked || !webDavConfigMatches" @click="uploadWebDavBackupManually">{{ busy === 'webdav-upload' ? '正在上传…' : '上传备份' }}</button>
        <button type="button" class="button danger-outline small" :disabled="!!busy || locked || !webDavConfigMatches" @click="restoreWebDavBackupManually">{{ busy === 'restore' ? '正在恢复…' : '从 WebDAV 恢复' }}</button>
      </div>
      <p v-if="webDavError" class="webdav-error">{{ webDavError }}</p>
    </section>
    </div>
  </section>
</template>

<style scoped>
.backup-settings-page{width:100%;min-width:0;color:var(--app-text)}
.backup-topbar{position:sticky;top:0;z-index:35;min-height:var(--ui-header-height);display:grid;grid-template-columns:44px minmax(0,1fr) 42px;align-items:center;gap:10px;padding-inline:var(--ui-header-gutter);border-bottom:1px solid var(--app-line);background:var(--app-header);backdrop-filter:blur(18px)}
.backup-page-title{min-width:0}
.backup-page-title strong{font-size:var(--ui-font-heading);color:var(--app-text);font-weight:750}
.backup-summary{display:flex;align-items:baseline;justify-content:space-between;flex-wrap:wrap;gap:var(--ui-space-2);padding-bottom:var(--ui-space-5);border-bottom:1px solid var(--app-line)}
.backup-summary>span:last-child{color:var(--app-muted);font-size:var(--ui-font-caption);overflow-wrap:anywhere}
.backup-local-section,.webdav-settings{padding-top:var(--ui-space-4)}
.backup-content h2{margin:0 0 var(--ui-space-2);font-size:var(--ui-font-body);font-weight:750}
.backup-local-section .settings-menu-row{padding-inline:0}
.backup-local-section .settings-menu-row:last-child{border-bottom:0}
.settings-menu-copy{display:grid;gap:var(--ui-space-1)}
.settings-menu-copy small{color:var(--app-muted);font-size:var(--ui-font-caption);line-height:1.5}
.webdav-settings{border-top:1px solid var(--app-line)}
.webdav-config-form{display:grid}
.webdav-setting-row{display:grid;grid-template-columns:minmax(0,1fr) minmax(140px,.8fr);align-items:center;gap:var(--ui-space-4);min-height:64px;padding:var(--ui-space-3) 0;border-bottom:1px solid var(--app-line)}
.backup-row-copy{display:flex;flex-direction:column;gap:4px;min-width:0}
.backup-row-copy strong{font-size:var(--ui-font-body);font-weight:650;color:var(--app-text)}
.backup-row-copy small{font-size:var(--ui-font-caption);color:var(--app-muted);line-height:1.45}
.webdav-setting-row input,.webdav-setting-row select{box-sizing:border-box;min-width:0;width:100%;min-height:var(--ui-control-height);padding:0 9px;border:1px solid var(--app-line);border-radius:var(--ui-radius);background:var(--app-surface);color:var(--app-text);font-size:var(--ui-font-body)}
.webdav-setting-row input:disabled,.webdav-setting-row select:disabled{opacity:.6}
.webdav-config-actions,.webdav-actions{display:flex;justify-content:flex-end;align-items:center;flex-wrap:wrap;gap:var(--ui-space-2);padding:var(--ui-space-3) 0}
.webdav-config-actions .button,.webdav-actions .button{min-height:var(--ui-control-height);font-size:var(--ui-font-body);font-weight:500}
.webdav-config-state{margin:0;padding:4px 0 8px;color:var(--app-muted);font-size:var(--ui-font-caption);line-height:1.5;overflow-wrap:anywhere}
.webdav-help{color:var(--app-muted);font-size:var(--ui-font-caption);margin:4px 0 16px}
.webdav-help summary{padding-block:4px;cursor:pointer}
.webdav-help p{margin:8px 0 0;line-height:1.5}
.webdav-actions{padding-bottom:0}
.webdav-auto-setting{grid-template-columns:minmax(0,1fr) 120px;border-top:1px solid var(--app-line)}
.webdav-draft-warning{display:flex;align-items:center;flex-wrap:wrap;gap:7px;margin:10px 0;padding:9px;border:1px solid var(--app-warning-line);border-radius:9px;background:var(--app-panel-soft);color:var(--app-warning);font-size:var(--ui-font-caption)}
.webdav-draft-warning button{padding:6px 8px;border:0;border-radius:7px;background:var(--app-surface);color:var(--app-accent);cursor:pointer}
.webdav-error{margin:0;padding:9px 0;color:var(--app-danger);font-size:var(--ui-font-caption);line-height:1.55}
.webdav-setting-row input:focus-visible,.webdav-setting-row select:focus-visible,.webdav-help summary:focus-visible{outline:2px solid var(--app-accent);outline-offset:2px}
@media(max-width:520px){
  .webdav-config-form .webdav-setting-row{grid-template-columns:minmax(0,1fr);gap:var(--ui-space-2)}
}
</style>

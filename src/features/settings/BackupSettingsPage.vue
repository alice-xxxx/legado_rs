<script setup lang="ts">
// 页面拥有备份 UI 与动作；阅读器只通过窄 session 暴露备份所需能力。
import { computed, ref, watch } from "vue";
import { backupBusy } from "./backupState";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { useBackupActions } from "./useBackupActions";
import { settings as sharedSettings } from "./settingsStore";
import router, { routeNames } from "../../router";
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
  void router.push({ name: routeNames.settings });
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
    <div class="backup-content">
    <header class="backup-hero">
      <span class="backup-hero-icon"><PrototypeIcon name="backup"/></span>
      <div class="backup-hero-copy">
        <strong>最近备份</strong><span>{{ lastBackup }}</span>
        <small>包含书架、阅读记录、书签、设置、缓存、来源与登录状态</small>
      </div>
      <button type="button" class="button primary small" :disabled="!!busy || locked"
        @click="createBackup">{{ busy === 'create' ? '正在备份…' : '立即备份' }}</button>
    </header>

    <section class="backup-local-section">
      <header><h3>本地</h3></header>
      <button type="button" class="backup-action-link" :disabled="!!busy || locked" @click="createBackup">
        <span class="backup-row-icon"><PrototypeIcon name="export"/></span>
        <span><strong>导出备份文件</strong><small>备份内容库、阅读数据和本地资源</small></span><PrototypeIcon name="right"/>
      </button>
      <button type="button" class="backup-action-link" :disabled="!!busy || locked" @click="restoreBackup">
        <span class="backup-row-icon"><PrototypeIcon name="import"/></span>
        <span><strong>从备份恢复</strong><small>选择已有备份，会替换当前本地数据</small></span><PrototypeIcon name="right"/>
      </button>
      <p class="restore-note">恢复会替换当前本地数据，完成后重新加载书架，请先确认备份文件来源。</p>
    </section>

    <section class="webdav-settings">
      <header class="webdav-heading">
        <div>
          <h3 id="webdav-backup-heading">WebDAV</h3>
          <p>远程同步与恢复</p>
        </div>
      </header>

      <form class="webdav-config-form" @submit.prevent="saveWebDavDraft">
        <label class="webdav-setting-row">
          <span class="backup-row-copy"><strong>地址</strong><small>远程目录 URL</small></span>
          <input v-model="webDavUrl" type="url" inputmode="url" autocomplete="url" maxlength="4096" placeholder="https://example.com/remote.php/dav/files/user/" :disabled="!!busy || locked" required />
        </label>
        <label class="webdav-setting-row">
          <span class="backup-row-copy"><strong>用户名</strong><small>WebDAV 账号</small></span>
          <input v-model="webDavUsername" autocomplete="username" maxlength="512" placeholder="WebDAV 用户名" :disabled="!!busy || locked" />
        </label>
        <label class="webdav-setting-row">
          <span class="backup-row-copy"><strong>密码</strong><small>密码不会回填到页面</small></span>
          <input v-model="webDavPassword" type="password" autocomplete="new-password" maxlength="4096" :placeholder="webDavConfig?.password ? sameWebDavIdentity ? '已保存；留空保持不变' : '更换目录或用户名时需重新输入密码' : '输入 WebDAV 密码'" :disabled="!!busy || locked" />
        </label>
        <div v-if="configRefreshConflict" class="webdav-draft-warning">
          已保存的 WebDAV 配置发生变化。当前输入仍保留，若要放弃草稿请点击重新读取表单。
          <button type="button" :disabled="!!busy || locked" @click="resetWebDavDraft">重新读取表单</button>
        </div>
        <div class="webdav-config-actions">
          <button type="submit" class="button primary small" :disabled="!!busy || locked || !webDavUrl.trim()">保存连接设置</button>
          <button v-if="webDavConfig?.password" type="button" class="text-button" :disabled="!!busy || locked || !webDavConfigMatches" @click="clearWebDavPassword">清除已保存密码</button>
        </div>
      </form>

      <p v-if="webDavConfig && webDavConfig.url" class="webdav-config-state">当前目录：{{ webDavConfig.url }}<span v-if="webDavConfig.username"> · 用户：{{ webDavConfig.username }}</span> · {{ webDavConfig?.password ? '密码已保存' : '未保存密码' }}</p>
      <p class="webdav-directory-note">远程目录必须已存在，应用不会创建目录或列出远程文件。密码不会回填；目录和用户名留空可保留已存密码，更换目录或用户名时留空会清除旧密码。</p>

      <div class="webdav-setting-row webdav-auto-setting">
        <label class="backup-row-copy" for="webdav-auto-interval"><strong>应用运行期间自动备份</strong><small>只备份已经保存的数据；应用关闭后不会继续运行。</small></label>
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
.backup-content{box-sizing:border-box;width:100%;margin:0 auto;padding:var(--ui-space-6) var(--ui-page-gutter) var(--ui-space-8);max-width:var(--app-content-width)}
.backup-hero{display:grid;grid-template-columns:44px minmax(0,1fr) auto;align-items:center;gap:12px;box-sizing:border-box;min-height:82px;padding:14px;border:1px solid var(--app-line);border-radius:14px;background:var(--app-surface)}
.backup-hero-icon{display:grid;place-items:center;width:40px;height:40px;border-radius:11px;background:var(--app-accent-soft);color:var(--app-accent)}
.backup-hero-icon :deep(svg){width:21px;height:21px}
.backup-hero-copy{display:flex;flex-direction:column;gap:3px;min-width:0}
.backup-hero-copy strong{font-size:var(--ui-font-body);color:var(--app-text)}
.backup-hero-copy>span{font-size:var(--ui-font-caption);color:var(--app-muted)}
.backup-hero-copy small{font-size:var(--ui-font-caption);color:var(--app-subtle);line-height:1.45}
.backup-hero>.button{justify-self:end;white-space:nowrap;min-height:36px;font-size:var(--ui-font-body)}
.backup-local-section{margin-top:22px}
.backup-local-section>header,.webdav-heading{display:flex;align-items:center;justify-content:space-between;gap:10px;margin:0 0 6px}
.backup-local-section>header h3,.webdav-heading h3{margin:0;color:var(--app-muted);font-size:var(--ui-font-body);font-weight:650}
.backup-local-section>header p,.webdav-heading p{margin:0;color:var(--app-subtle);font-size:var(--ui-font-caption);line-height:1.45}
.backup-action-link{width:100%;display:grid;grid-template-columns:37px minmax(0,1fr) 16px;gap:10px;align-items:center;min-height:58px;padding:6px;border:0;border-bottom:1px solid var(--app-line);background:transparent;text-align:left;cursor:pointer}
.backup-action-link:disabled{opacity:.5;cursor:default}
.backup-row-icon{display:grid;place-items:center;width:36px;height:36px;border-radius:10px;background:var(--app-accent-soft);color:var(--app-accent)}
.backup-row-icon :deep(svg){width:19px;height:19px}
.backup-action-link>span:nth-child(2){display:flex;flex-direction:column;gap:3px}
.backup-action-link strong{font-size:var(--ui-font-body);color:var(--app-text)}
.backup-action-link small{font-size:var(--ui-font-caption);color:var(--app-subtle);line-height:1.45}
.backup-action-link>.prototype-icon{width:16px;height:16px;color:var(--app-subtle)}
.restore-note{margin:10px 0 0;color:var(--app-warning);font-size:var(--ui-font-caption);line-height:1.55}
.webdav-settings{margin-top:22px;padding:15px 0;border-top:1px solid var(--app-line);background:transparent}
.webdav-heading{align-items:center}
.webdav-heading>div{display:flex;align-items:center;gap:8px}
.webdav-heading h3{font-size:var(--ui-font-caption)}
.webdav-heading .text-button{font-size:var(--ui-font-body)}
.webdav-config-form{display:grid;margin-top:12px}
.webdav-setting-row{display:grid;grid-template-columns:minmax(0,1fr);align-items:start;gap:6px;min-height:0;padding:9px 0;border-bottom:1px solid var(--app-line)}
.backup-row-copy{display:flex;flex-direction:column;gap:4px;min-width:0}
.backup-row-copy strong{font-size:var(--ui-font-body);color:var(--app-text)}
.backup-row-copy small{font-size:var(--ui-font-caption);color:var(--app-subtle);line-height:1.45}
.webdav-setting-row input,.webdav-setting-row select{box-sizing:border-box;min-width:0;width:min(100%,520px);min-height:33px;padding:0 9px;border:1px solid var(--app-line);border-radius:8px;background:var(--app-surface);color:var(--app-text);font-size:var(--ui-font-body)}
.webdav-setting-row input:disabled,.webdav-setting-row select:disabled{opacity:.6}
.webdav-config-actions,.webdav-actions{display:flex;justify-content:flex-end;align-items:center;flex-wrap:wrap;gap:8px;padding:12px 0}
.webdav-config-actions .button,.webdav-actions .button{min-height:33px;font-size:var(--ui-font-body)}
.webdav-config-state,.webdav-directory-note,.backup-status-note{margin:0;padding:8px 0;color:var(--app-muted);font-size:var(--ui-font-caption);line-height:1.65;overflow-wrap:anywhere}
.webdav-config-state{color:var(--app-accent-ink)}
.webdav-directory-note{border-bottom:1px solid var(--app-line)}
.webdav-actions{border-top:1px solid var(--app-line);margin-top:12px;padding-bottom:0}
.webdav-auto-setting select{min-width:125px}
.webdav-draft-warning{display:flex;align-items:center;flex-wrap:wrap;gap:7px;margin:10px 0;padding:9px;border:1px solid var(--app-warning-line);border-radius:9px;background:var(--app-panel-soft);color:var(--app-warning);font-size:var(--ui-font-caption)}
.webdav-draft-warning button{padding:6px 8px;border:0;border-radius:7px;background:var(--app-surface);color:var(--app-accent);cursor:pointer}
.webdav-error{margin:0;padding:9px 0;color:var(--app-danger);font-size:var(--ui-font-caption);line-height:1.55}
@media(max-width:660px){
  .backup-topbar{padding-inline:var(--ui-header-gutter)}
  .backup-content{width:100%}
  .backup-hero{grid-template-columns:40px minmax(0,1fr);gap:10px}
  .backup-hero>.button{grid-column:1/-1;justify-self:stretch}
  .webdav-heading{flex-wrap:wrap}
  .webdav-heading .text-button{margin-left:auto}
  .webdav-setting-row input,.webdav-setting-row select{width:100%}
}
</style>

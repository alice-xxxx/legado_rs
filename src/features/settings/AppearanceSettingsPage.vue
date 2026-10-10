<script setup lang="ts">
// 本页拥有阅读设置草稿与保存流程；Rust 负责规范化、持久化并广播新设置。
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { onBeforeRouteLeave } from "vue-router";
import { appBootstrap } from "../../api/core";
import { readResource } from "../../api/resources";
import { clearReaderBackgroundImage, pickReaderBackgroundImage, saveSettings } from "../../api/settings";
import type { AppBootstrap, AppSettingsResource, ReaderSettings, ResourceDescriptor } from "../../api/types";
import { defaultSettings, normalizeSettings } from "./settingsDefaults";
import { backupBusy } from "./backupState";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { notify } from "../../app/notifications";
import router, { routeNames } from "../../router";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import BackButton from "../../ui/BackButton.vue";

type AppearanceTheme = ReaderSettings["theme"];
type AppearanceSettingKey = "backgroundImageOpacity" | "fontSizePx" | "lineHeight" | "fontFamily" | "preloadCount" | "verticalScroll" | "audioPlaybackRate" | "videoPlaybackRate" | "audioSkipSeconds";

const settings = ref<AppSettingsResource>(structuredClone(defaultSettings));
const readerSettings = computed(() => settings.value.reader);
const savedSnapshot = ref("");
const loading = ref(true);
const dirty = computed(() => !loading.value && !loadError.value && JSON.stringify(settings.value) !== savedSnapshot.value);
const saving = ref(false);
const backgroundBusy = ref(false);
const backgroundError = ref("");
const loadError = ref("");
const operationBlocked = computed(() => backupBusy.value !== null || shelfBatchRecoveryRequired.value);
const blocked = computed(() => operationBlocked.value || loading.value || !!loadError.value || saving.value);
const playbackRates = [0.5, 0.75, 1, 1.25, 1.5, 1.75, 2, 2.25, 2.5, 2.75, 3] as const;
const themes: ReadonlyArray<readonly [AppearanceTheme, string]> = [
  ["paper", "纸张"], ["sepia", "护眼"], ["dark", "夜间"], ["system", "系统"],
];
let disposed = false;
let restoreRevision = 0;
let loadRevision = 0;
let settingsChangeRevision = 0;
let pendingSave: ReturnType<typeof setTimeout> | null = null;
const unlisteners: UnlistenFn[] = [];

function returnToSettings(): void {
  void router.push({ name: routeNames.settings });
}

async function applySettingsDescriptor(descriptor: ResourceDescriptor): Promise<void> {
  const revision = ++loadRevision;
  const restoreAtStart = restoreRevision;
  const changeAtStart = settingsChangeRevision;
  loading.value = true;
  try {
    const next = normalizeSettings(await readResource<AppSettingsResource>(descriptor));
    if (disposed || revision !== loadRevision || restoreAtStart !== restoreRevision) return;
    if (changeAtStart === settingsChangeRevision && !saving.value) {
      settings.value = next;
      savedSnapshot.value = JSON.stringify(next);
    }
    loadError.value = "";
  } catch (error) {
    if (!disposed && revision === loadRevision && restoreAtStart === restoreRevision) {
      loadError.value = error instanceof Error ? error.message : String(error);
    }
  } finally {
    if (!disposed && revision === loadRevision && restoreAtStart === restoreRevision) loading.value = false;
  }
}

async function loadSettings(snapshot?: AppBootstrap): Promise<void> {
  const revision = ++loadRevision;
  const restoreAtStart = restoreRevision;
  loading.value = true;
  try {
    const current = snapshot ?? await appBootstrap();
    const next = normalizeSettings(await readResource<AppSettingsResource>(current.settings));
    if (disposed || revision !== loadRevision || restoreAtStart !== restoreRevision) return;
    settings.value = next;
    savedSnapshot.value = JSON.stringify(next);
    loadError.value = "";
  } catch (error) {
    if (!disposed && revision === loadRevision && restoreAtStart === restoreRevision) {
      loadError.value = error instanceof Error ? error.message : String(error);
    }
  } finally {
    if (!disposed && revision === loadRevision && restoreAtStart === restoreRevision) loading.value = false;
  }
}

function scheduleSave(): void {
  settingsChangeRevision += 1;
  if (pendingSave) clearTimeout(pendingSave);
  pendingSave = setTimeout(() => { void persistSettings(); }, 450);
}

async function persistSettings(): Promise<boolean> {
  if (operationBlocked.value || loading.value || !!loadError.value || saving.value) return false;
  if (pendingSave) clearTimeout(pendingSave);
  pendingSave = null;
  const restoreAtStart = restoreRevision;
  const changeAtStart = settingsChangeRevision;
  saving.value = true;
  try {
    const normalized = normalizeSettings(settings.value);
    // Background image URLs are runtime resource addresses and are saved by the image picker.
    delete normalized.reader.backgroundImageSrc;
    const descriptor = await saveSettings(normalized);
    const persisted = normalizeSettings(await readResource<AppSettingsResource>(descriptor));
    if (disposed || restoreAtStart !== restoreRevision) return false;
    if (changeAtStart === settingsChangeRevision) {
      settings.value = persisted;
      savedSnapshot.value = JSON.stringify(persisted);
    }
    notify("阅读设置已保存。");
    return true;
  } catch (error) {
    if (!disposed && restoreAtStart === restoreRevision) {
      notify(`阅读设置保存失败：${error instanceof Error ? error.message : String(error)}`, "error");
    }
    return false;
  } finally {
    if (restoreAtStart === restoreRevision) saving.value = false;
  }
}

onBeforeRouteLeave(async () => {
  if (loading.value || !!loadError.value || !dirty.value || operationBlocked.value) return true;
  return await persistSettings();
});

watch(operationBlocked, (locked) => {
  if (locked) {
    if (pendingSave) clearTimeout(pendingSave);
    pendingSave = null;
  } else if (dirty.value) {
    scheduleSave();
  }
});

function changeSetting<K extends AppearanceSettingKey>(key: K, value: ReaderSettings[K]): void {
  settings.value = { ...settings.value, reader: { ...settings.value.reader, [key]: value } };
  scheduleSave();
}

function changeTheme(theme: AppearanceTheme): void {
  const colors: Record<AppearanceTheme, { textColor: string; backgroundColor: string }> = {
    paper: { textColor: "#3f3b34", backgroundColor: "#f7f3e9" },
    sepia: { textColor: "#4e4131", backgroundColor: "#eee1c8" },
    dark: { textColor: "#d1d0cb", backgroundColor: "#17191c" },
    system: { textColor: "#252525", backgroundColor: "#ffffff" },
  };
  settings.value = { ...settings.value, reader: { ...settings.value.reader, theme, ...colors[theme] } };
  scheduleSave();
}

async function chooseBackground(): Promise<void> {
  if (operationBlocked.value || backgroundBusy.value) return;
  backgroundBusy.value = true;
  backgroundError.value = "";
  try {
    const result = await pickReaderBackgroundImage();
    if (result.cancelled) return;
    await applySettingsDescriptor(result.settings);
    notify("阅读背景图片已保存。");
  } catch (error) {
    backgroundError.value = error instanceof Error ? error.message : String(error);
  } finally {
    backgroundBusy.value = false;
  }
}

async function clearBackground(): Promise<void> {
  if (operationBlocked.value || backgroundBusy.value) return;
  backgroundBusy.value = true;
  backgroundError.value = "";
  try {
    await applySettingsDescriptor(await clearReaderBackgroundImage());
    notify("阅读背景图片已清除。");
  } catch (error) {
    backgroundError.value = error instanceof Error ? error.message : String(error);
  } finally {
    backgroundBusy.value = false;
  }
}

onMounted(async () => {
  const unlistenSettings = await listen<ResourceDescriptor>("settings-updated", (event) => {
    void applySettingsDescriptor(event.payload);
  });
  if (disposed) { unlistenSettings(); return; }
  unlisteners.push(unlistenSettings);
  const unlistenRestore = await listen<AppBootstrap>("app-state-updated", (event) => {
    restoreRevision += 1;
    loadRevision += 1;
    settingsChangeRevision += 1;
    if (pendingSave) clearTimeout(pendingSave);
    pendingSave = null;
    void loadSettings(event.payload);
  });
  if (disposed) { unlistenRestore(); return; }
  unlisteners.push(unlistenRestore);
  await loadSettings();
});

onBeforeUnmount(() => {
  disposed = true;
  restoreRevision += 1;
  loadRevision += 1;
  if (pendingSave) clearTimeout(pendingSave);
  for (const unlisten of unlisteners) unlisten();
});
</script>

<template>
  <section class="appearance-settings-page my-settings-subpage">
    <header class="appearance-topbar">
      <BackButton label="返回我的" @click="returnToSettings" />
      <div class="appearance-page-title"><strong>阅读与播放</strong></div>
      <span></span>
    </header>
    <p v-if="loadError" class="appearance-error">
      阅读设置载入失败：{{ loadError }}
      <button type="button" class="text-button" :disabled="loading" @click="loadSettings()">重试</button>
    </p>
    <div class="appearance-workspace">

      <main class="appearance-settings-stack">
      <section class="appearance-workspace-group">
        <header><h2>文字阅读</h2></header>
    <div class="appearance-setting-row">
      <div class="appearance-setting-copy">
        <label for="reader-font-size">字体大小</label>
        <small>当前 {{ readerSettings.fontSizePx }} px</small>
      </div>
      <div class="appearance-range-control">
        <input
          id="reader-font-size"
          type="range"
          min="12"
          max="36"
          :value="readerSettings.fontSizePx"
          :disabled="saving || blocked"
          @input="changeSetting('fontSizePx', Number(($event.target as HTMLInputElement).value))"
        />
      </div>
    </div>

    <div class="appearance-setting-row">
      <div class="appearance-setting-copy">
        <label for="reader-line-height">行间距</label>
        <small>{{ readerSettings.lineHeight.toFixed(1) }}</small>
      </div>
      <input
        id="reader-line-height"
        class="appearance-single-range"
        type="range"
        min="1.2"
        max="2.8"
        step="0.1"
        :value="readerSettings.lineHeight"
        :disabled="saving || blocked"
        @input="changeSetting('lineHeight', Number(($event.target as HTMLInputElement).value))"
      />
    </div>

    <div class="appearance-setting-row">
      <div class="appearance-setting-copy">
        <label for="reader-font-family">正文字体</label>
        <small>只改变显示效果</small>
      </div>
      <select
        id="reader-font-family"
        :value="readerSettings.fontFamily"
        :disabled="saving || blocked"
        @change="changeSetting('fontFamily', ($event.target as HTMLSelectElement).value)"
      >
        <option value="serif">衬线字体</option>
        <option value="sans">无衬线字体</option>
        <option value="system">系统字体</option>
        <option value="mono">等宽字体</option>
      </select>
    </div>


    <div class="appearance-setting-row">
      <div class="appearance-setting-copy">
        <label for="reader-turn-style">翻页方式</label>
        <small>正文左右翻页或上下滚动</small>
      </div>
      <select id="reader-turn-style" :value="readerSettings.verticalScroll ? 'scroll' : 'page'"
        :disabled="saving || blocked" @change="changeSetting('verticalScroll', ($event.target as HTMLSelectElement).value === 'scroll')">
        <option value="page">左右翻页</option><option value="scroll">连续滚动</option>
      </select>
    </div>
      </section>
      <section class="appearance-workspace-group">
        <header><h2>主题</h2></header>
    <div class="appearance-setting-row appearance-theme-row">
      <div class="appearance-theme-options">
        <button
          v-for="theme in themes"
          :key="theme[0]"
          type="button"
          class="appearance-theme-option"
          :class="[`theme-${theme[0]}`, { selected: readerSettings.theme === theme[0] }]"
          :disabled="saving || blocked"
          @click="changeTheme(theme[0])"
        ><span></span>{{ theme[1] }}</button>
      </div>
    </div>


      </section>
      <section class="appearance-workspace-group">
        <header><h2>音频</h2></header>
    <div class="appearance-setting-row">
      <div class="appearance-setting-copy">
        <label for="reader-default-audio-speed">音频默认倍速</label>
        <small>播放音频章节时使用的默认速度</small>
      </div>
      <select id="reader-default-audio-speed" :value="readerSettings.audioPlaybackRate"
        :disabled="saving || blocked" @change="changeSetting('audioPlaybackRate', Number(($event.target as HTMLSelectElement).value))">
        <option v-for="speed in playbackRates" :key="speed" :value="speed">{{ speed }}×</option>
      </select>
    </div>
    <div class="appearance-setting-row">
      <div class="appearance-setting-copy">
        <label for="reader-audio-skip">跳过时长</label>
        <small>音频播放页快进与后退按钮，系统媒体控制也使用此时长</small>
      </div>
      <select id="reader-audio-skip" :value="readerSettings.audioSkipSeconds"
        :disabled="saving || blocked" @change="changeSetting('audioSkipSeconds', Number(($event.target as HTMLSelectElement).value))">
        <option :value="10">10 秒</option>
        <option :value="15">15 秒</option>
        <option :value="30">30 秒</option>
      </select>
    </div>
      </section>
      <section class="appearance-workspace-group">
        <header><h2>视频</h2></header>
        <div class="appearance-setting-row">
          <div class="appearance-setting-copy">
            <label for="reader-default-video-speed">默认倍速</label>
            <small>打开新视频时使用；与音频倍速分开保存</small>
          </div>
          <select id="reader-default-video-speed"
            :value="readerSettings.videoPlaybackRate" :disabled="saving || blocked"
            @change="changeSetting('videoPlaybackRate', Number(($event.target as HTMLSelectElement).value))">
            <option v-for="speed in playbackRates" :key="speed" :value="speed">{{ speed }}×</option>
          </select>
        </div>
        <p class="appearance-video-note">清晰度和字幕取决于内容来源及播放器实际提供的轨道；可用的清晰度在视频播放页面选择。</p>
      </section>
      <details class="appearance-advanced">
        <summary>更多阅读选项 <span>预读与自定义背景</span><PrototypeIcon name="chevron-right"/></summary>
      <section class="appearance-workspace-group">
        <header><h2>预读</h2></header>
    <div class="appearance-setting-row">
      <div class="appearance-setting-copy">
        <label for="preload-count">预读章节</label>
        <small>提前准备 {{ readerSettings.preloadCount }} 章</small>
      </div>
      <div class="appearance-range-control">
        <input
          id="preload-count"
          type="range"
          min="1"
          max="20"
          step="1"
          :value="readerSettings.preloadCount"
          :disabled="saving || blocked"
          @input="changeSetting('preloadCount', Number(($event.target as HTMLInputElement).value))"
        />
      </div>
    </div>




      </section>
      <section class="appearance-workspace-group">
        <header><h2>阅读背景</h2></header>
    <div class="appearance-setting-row">
      <div class="appearance-setting-copy">
        <label>阅读背景图片</label>
        <small>选择 JPEG、PNG 或 WebP 图片</small>
      </div>
      <div class="appearance-image-controls">
        <img v-if="readerSettings.backgroundImageSrc" :src="readerSettings.backgroundImageSrc"/>
        <span v-else class="appearance-unset">未设置</span>
        <button
          type="button"
          class="button secondary small"
          :disabled="backgroundBusy || saving || blocked"
          @click="chooseBackground"
        >{{ backgroundBusy ? '处理中…' : readerSettings.backgroundImageSrc ? '更换图片' : '选择图片' }}</button>
        <button
          v-if="readerSettings.backgroundImageSrc"
          type="button"
          class="text-button"
          :disabled="backgroundBusy || saving || blocked"
          @click="clearBackground"
        >清除</button>
      </div>
    </div>

    <div class="appearance-setting-row">
      <div class="appearance-setting-copy">
        <label for="reader-background-image-opacity">背景图片强度</label>
        <small>低强度能让正文保持清晰</small>
      </div>
      <div class="appearance-range-control">
        <input
          id="reader-background-image-opacity"
          type="range"
          min="0"
          max="100"
          step="1"
          :value="readerSettings.backgroundImageOpacity"
          :disabled="backgroundBusy || saving || blocked"
          @input="changeSetting('backgroundImageOpacity', Number(($event.target as HTMLInputElement).value))"
        />
        <strong>{{ readerSettings.backgroundImageOpacity }}%</strong>
      </div>
    </div>
    <p v-if="backgroundError" class="appearance-error">{{ backgroundError }}</p>


      </section>

      </details>
    <div class="appearance-save-row">
      <span>{{ saving ? '保存中…' : '更改会自动保存。' }}</span>
      <button class="button secondary small" :disabled="saving || blocked" @click="persistSettings">{{ saving ? '保存中…' : '立即保存' }}</button>
    </div>

      </main>
    </div>
  </section>
</template>

<style scoped>
.appearance-settings-page{width:100%;min-width:0;color:#353643}
.appearance-topbar{position:sticky;top:0;z-index:35;display:grid;grid-template-columns:44px minmax(0,1fr) 42px;align-items:center;gap:10px;box-sizing:border-box;height:64px;padding:0 max(18px,calc((100% - 1180px)/2));border-bottom:1px solid var(--app-line);background:#f6f7f9ec;backdrop-filter:blur(18px)}
.appearance-page-title{min-width:0}
.appearance-page-title strong{font-size:14px;color:#242631;font-weight:750}
.appearance-workspace{box-sizing:border-box;width:min(820px,calc(100% - 36px));margin:0 auto;padding:24px 0 54px}
.appearance-settings-stack{display:grid;gap:0;min-width:0}
.appearance-workspace-group{padding:15px 0;border:0;border-top:1px solid var(--app-line);background:transparent}
.appearance-workspace-group:first-child{border-top:0;padding-top:0}
.appearance-workspace-group>header{margin:0 0 8px}
.appearance-workspace-group h2{margin:0;color:#343642;font-size:11px;font-weight:750}
.appearance-setting-row{box-sizing:border-box;display:grid;grid-template-columns:minmax(0,1fr) minmax(140px,.8fr);align-items:center;gap:10px;min-height:58px;padding:9px 0;border-bottom:1px solid #eef0f3}
.appearance-workspace-group .appearance-setting-row:last-child{border-bottom:0}
.appearance-setting-copy{display:flex;flex-direction:column;gap:2px;min-width:0}
.appearance-setting-copy label{font-size:10px;font-weight:650;color:#41434d}
.appearance-setting-copy small{font-size:8px;line-height:1.45;color:#999da5}
.appearance-setting-row>select{justify-self:end;width:min(100%,250px);min-height:33px;padding:0 8px;border:1px solid var(--app-line);border-radius:8px;background:#fff;color:#555864;font-size:9px}
.appearance-range-control{display:flex;align-items:center;gap:8px;min-width:0}
.appearance-range-control input,.appearance-single-range{width:100%;min-width:0;accent-color:var(--app-accent)}
.appearance-range-control strong{min-width:38px;text-align:right;color:#5955a9;font-size:10px;font-variant-numeric:tabular-nums}
.appearance-image-controls{display:flex;align-items:center;justify-content:flex-end;flex-wrap:wrap;gap:8px}
.appearance-image-controls img{width:42px;height:42px;object-fit:cover;border-radius:8px}
.appearance-unset{font-size:10px;color:#9699a3}
.appearance-theme-row{display:block;padding:8px 0 2px}
.appearance-theme-options{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:7px;width:100%}
.appearance-theme-option{min-width:0;display:flex;flex-direction:column;gap:5px;padding:7px;border:1px solid #e5e6ed;border-radius:10px;background:#fff;color:#777c85;font-size:8px;text-align:center;cursor:pointer}
.appearance-theme-option>span{display:block;width:100%;height:50px;border:0;border-radius:7px;background:#f4eee2}
.appearance-theme-option.theme-sepia>span{background:#e5cfaa}
.appearance-theme-option.theme-dark>span{background:#25272c}
.appearance-theme-option.theme-system>span{background:linear-gradient(135deg,#f4eee2 50%,#25272c 50%)}
.appearance-theme-option.selected{border-color:#dcd5fa;color:var(--app-accent);background:#f4f1ff;font-weight:750}
.appearance-theme-option.selected>span{box-shadow:0 0 0 1px #b9aef0}
.appearance-video-note{margin:8px 0;color:#9296a1;font-size:9px;line-height:1.65}
.appearance-advanced{border-top:1px solid var(--app-line)}
.appearance-advanced>summary{display:flex;align-items:center;gap:9px;min-height:43px;color:#4f5360;font-size:10px;font-weight:650;cursor:pointer;list-style:none}
.appearance-advanced>summary::-webkit-details-marker{display:none}
.appearance-advanced>summary span{margin-left:auto;color:#9a9da6;font-size:9px;font-weight:400}
.appearance-advanced>summary :deep(svg){width:16px;height:16px;transform:rotate(90deg)}
.appearance-advanced[open]>summary :deep(svg){transform:rotate(-90deg)}
.appearance-advanced .appearance-workspace-group:first-of-type{border-top:1px solid var(--app-line);padding-top:15px}
.appearance-error{margin:0;padding:10px 0;color:#ac5458;font-size:11px}
.appearance-save-row{display:flex;align-items:center;justify-content:space-between;gap:12px;padding:12px 0;color:#868b97;font-size:10px}
@media(max-width:660px){
  .appearance-topbar{padding:0 18px}
  .appearance-workspace{width:calc(100% - 36px)}
  .appearance-setting-row{grid-template-columns:minmax(0,1fr) minmax(110px,.8fr);gap:8px}
  .appearance-theme-options{gap:5px}
  .appearance-theme-option{padding:5px}
  .appearance-theme-option>span{height:40px}
}
@media(max-width:380px){
 .appearance-setting-row{grid-template-columns:1fr;gap:7px}
 .appearance-setting-row>select{justify-self:start;width:100%}
}
</style>

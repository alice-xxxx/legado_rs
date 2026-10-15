<script setup lang="ts">
// 本页直接修改全局响应式设置，由 settingsStore 自动保存。
import { computed, ref } from "vue";
import { pickReaderBackgroundImage } from "../../api/settings";
import type { ReaderSettings } from "../../api/types";
import { settings } from "./settingsStore";
import { isReadingMode, readingModes } from "./readerModes";
import { appThemes, readingThemes, resolvedReadingTheme, setReaderTheme } from "./themeState";
import { backupBusy } from "./backupState";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { notify } from "../../app/notifications";
import { routeNames } from "../../router";
import { returnToRoute } from "../../app/routeReturn";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import BackButton from "../../ui/BackButton.vue";

type AppearanceSettingKey = "backgroundImageOpacity" | "fontSizePx" | "lineHeight" | "fontFamily" | "preloadCount" | "audioPlaybackRate" | "videoPlaybackRate" | "audioSkipSeconds";

const backgroundBusy = ref(false);
const backgroundError = ref("");
const operationBlocked = computed(() => backupBusy.value !== null || shelfBatchRecoveryRequired.value);
const blocked = operationBlocked;
const playbackRates = [0.5, 0.75, 1, 1.25, 1.5, 1.75, 2, 2.25, 2.5, 2.75, 3] as const;
function returnToSettings(): void {
  returnToRoute(routeNames.settings);
}

function changeSetting<K extends AppearanceSettingKey>(key: K, value: ReaderSettings[K]): void {
  settings.value.reader[key] = value;
}

function changeReaderMode(event: Event): void {
  const mode = (event.target as HTMLSelectElement).value;
  if (isReadingMode(mode)) settings.value.reader.readingMode = mode;
}

async function chooseBackground(): Promise<void> {
  if (operationBlocked.value || backgroundBusy.value) return;
  backgroundBusy.value = true;
  backgroundError.value = "";
  try {
    const result = await pickReaderBackgroundImage();
    if (result.cancelled) return;
    settings.value.reader.backgroundImageSrc = result.image.src;
    notify("阅读背景图片已保存。");
  } catch (error) {
    backgroundError.value = error instanceof Error ? error.message : String(error);
  } finally {
    backgroundBusy.value = false;
  }
}

function clearBackground(): void {
  if (operationBlocked.value || backgroundBusy.value) return;
  delete settings.value.reader.backgroundImageSrc;
  backgroundError.value = "";
  notify("阅读背景图片已清除。");
}

</script>

<template>
  <section class="appearance-settings-page my-settings-subpage">
    <header class="appearance-topbar safe-area-top">
      <BackButton label="返回我的" @click="returnToSettings" />
      <div class="appearance-page-title"><strong>阅读与播放</strong></div>
      <span></span>
    </header>
    <div class="appearance-workspace">

      <main class="appearance-settings-stack">
      <section class="appearance-workspace-group">
        <header><h2>应用主题</h2></header>
    <div class="appearance-setting-row appearance-theme-row">
      <div class="appearance-theme-options app-theme-options">
        <button
          v-for="theme in appThemes"
          :key="theme[0]"
          type="button"
          class="appearance-theme-option"
          :class="[`theme-${theme[0]}`, { selected: settings.theme === theme[0] }]"
          :aria-pressed="settings.theme === theme[0]"
          :disabled="blocked"
          @click="settings.theme = theme[0]"
        ><span></span>{{ theme[1] }}</button>
      </div>
    </div>
    <template v-if="settings.theme !== 'system'">
      <header class="reading-theme-heading"><h2>{{ settings.theme === 'light' ? '白天' : '夜间' }}阅读主题</h2>
        <small>切换应用主题时，同时使用这里保存的阅读配色。</small></header>
      <div class="appearance-theme-options">
        <button v-for="theme in readingThemes" :key="theme[0]" type="button"
          class="appearance-theme-option" :class="[`theme-${theme[0]}`, { selected: resolvedReadingTheme === theme[0] }]"
          :aria-pressed="resolvedReadingTheme === theme[0]" :disabled="blocked" @click="setReaderTheme(theme[0])">
          <span></span>{{ theme[1] }}
        </button>
      </div>
    </template>
    <p v-else class="appearance-theme-note">随系统自动切换应用配色和对应的阅读主题。</p>
      </section>
      <section class="appearance-workspace-group">
        <header><h2>文字阅读</h2></header>
    <div class="appearance-setting-row">
      <div class="appearance-setting-copy">
        <label for="reader-font-size">字体大小</label>
        <small>当前 {{ settings.reader.fontSizePx }} px</small>
      </div>
      <div class="appearance-range-control">
        <input
          id="reader-font-size"
          type="range"
          min="12"
          max="36"
          :value="settings.reader.fontSizePx"
          :disabled="blocked"
          @input="changeSetting('fontSizePx', Number(($event.target as HTMLInputElement).value))"
        />
      </div>
    </div>

    <div class="appearance-setting-row">
      <div class="appearance-setting-copy">
        <label for="reader-line-height">行间距</label>
        <small>{{ settings.reader.lineHeight.toFixed(1) }}</small>
      </div>
      <input
        id="reader-line-height"
        class="appearance-single-range"
        type="range"
        min="1.2"
        max="2.8"
        step="0.1"
        :value="settings.reader.lineHeight"
        :disabled="blocked"
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
        :value="settings.reader.fontFamily"
        :disabled="blocked"
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
        <label for="reader-turn-style">阅读方式</label>
        <small>分页切换效果或上下连续滚动</small>
      </div>
      <select id="reader-turn-style" :value="settings.reader.readingMode ?? 'cover'"
        :disabled="blocked" @change="changeReaderMode">
        <option v-for="mode in readingModes" :key="mode.value" :value="mode.value">{{ mode.label }}</option>
      </select>
    </div>
      </section>
      <section class="appearance-workspace-group">
        <header><h2>音频</h2></header>
    <div class="appearance-setting-row">
      <div class="appearance-setting-copy">
        <label for="reader-default-audio-speed">音频默认倍速</label>
        <small>播放音频章节时使用的默认速度</small>
      </div>
      <select id="reader-default-audio-speed" :value="settings.reader.audioPlaybackRate"
        :disabled="blocked" @change="changeSetting('audioPlaybackRate', Number(($event.target as HTMLSelectElement).value))">
        <option v-for="speed in playbackRates" :key="speed" :value="speed">{{ speed }}×</option>
      </select>
    </div>
    <div class="appearance-setting-row">
      <div class="appearance-setting-copy">
        <label for="reader-audio-skip">跳过时长</label>
        <small>音频播放页快进与后退按钮，系统媒体控制也使用此时长</small>
      </div>
      <select id="reader-audio-skip" :value="settings.reader.audioSkipSeconds"
        :disabled="blocked" @change="changeSetting('audioSkipSeconds', Number(($event.target as HTMLSelectElement).value))">
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
            :value="settings.reader.videoPlaybackRate" :disabled="blocked"
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
        <small>提前准备 {{ settings.reader.preloadCount }} 章</small>
      </div>
      <div class="appearance-range-control">
        <input
          id="preload-count"
          type="range"
          min="1"
          max="20"
          step="1"
          :value="settings.reader.preloadCount"
          :disabled="blocked"
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
        <img v-if="settings.reader.backgroundImageSrc" :src="settings.reader.backgroundImageSrc"/>
        <span v-else class="appearance-unset">未设置</span>
        <button
          type="button"
          class="button secondary small"
          :disabled="backgroundBusy || blocked"
          @click="chooseBackground"
        >{{ backgroundBusy ? '处理中…' : settings.reader.backgroundImageSrc ? '更换图片' : '选择图片' }}</button>
        <button
          v-if="settings.reader.backgroundImageSrc"
          type="button"
          class="text-button"
          :disabled="backgroundBusy || blocked"
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
          :value="settings.reader.backgroundImageOpacity"
          :disabled="backgroundBusy || blocked"
          @input="changeSetting('backgroundImageOpacity', Number(($event.target as HTMLInputElement).value))"
        />
        <strong>{{ settings.reader.backgroundImageOpacity }}%</strong>
      </div>
    </div>
    <p v-if="backgroundError" class="appearance-error">{{ backgroundError }}</p>


      </section>

      </details>

      </main>
    </div>
  </section>
</template>

<style scoped>
.appearance-settings-page{width:100%;min-width:0;color:var(--app-text)}
.appearance-topbar{position:sticky;top:0;z-index:35;display:grid;grid-template-columns:44px minmax(0,1fr) 42px;align-items:center;gap:10px;min-height:var(--ui-header-height);padding-inline:var(--ui-header-gutter);border-bottom:1px solid var(--app-line);background:var(--app-header);backdrop-filter:blur(18px)}
.appearance-page-title{min-width:0}
.appearance-page-title strong{font-size:var(--ui-font-heading);color:var(--app-text);font-weight:750}
.appearance-workspace{box-sizing:border-box;width:100%;margin:0 auto;padding:var(--ui-space-6) var(--ui-page-gutter) var(--ui-space-8);max-width:var(--app-content-width)}
.appearance-settings-stack{display:grid;gap:0;min-width:0}
.appearance-workspace-group{padding:15px 0;border:0;border-top:1px solid var(--app-line);background:transparent}
.appearance-workspace-group:first-child{border-top:0;padding-top:0}
.appearance-workspace-group>header{margin:0 0 8px}
.appearance-workspace-group h2{margin:0;color:var(--app-text);font-size:var(--ui-font-body);font-weight:750}
.appearance-setting-row{box-sizing:border-box;display:grid;grid-template-columns:minmax(0,1fr) minmax(140px,.8fr);align-items:center;gap:10px;min-height:58px;padding:9px 0;border-bottom:1px solid var(--app-line)}
.appearance-workspace-group .appearance-setting-row:last-child{border-bottom:0}
.appearance-setting-copy{display:flex;flex-direction:column;gap:2px;min-width:0}
.appearance-setting-copy label{font-size:var(--ui-font-body);font-weight:650;color:var(--app-text)}
.appearance-setting-copy small{font-size:var(--ui-font-caption);line-height:1.45;color:var(--app-subtle)}
.appearance-setting-row>select{justify-self:end;width:min(100%,250px);min-height:33px;padding:0 8px;border:1px solid var(--app-line);border-radius:8px;background:var(--app-surface);color:var(--app-muted);font-size:var(--ui-font-body)}
.appearance-range-control{display:flex;align-items:center;gap:8px;min-width:0}
.appearance-range-control input,.appearance-single-range{width:100%;min-width:0;accent-color:var(--app-accent)}
.appearance-range-control strong{min-width:38px;text-align:right;color:var(--app-accent-ink);font-size:var(--ui-font-caption);font-variant-numeric:tabular-nums}
.appearance-image-controls{display:flex;align-items:center;justify-content:flex-end;flex-wrap:wrap;gap:8px}
.appearance-image-controls img{width:42px;height:42px;object-fit:cover;border-radius:8px}
.appearance-unset{font-size:var(--ui-font-caption);color:var(--app-subtle)}
.appearance-theme-row{display:block;padding:8px 0 2px}
.appearance-theme-options{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:7px;width:100%}
.app-theme-options{grid-template-columns:repeat(3,minmax(0,1fr))}
.reading-theme-heading{margin:16px 0 8px}
.reading-theme-heading small,.appearance-theme-note{color:var(--app-muted);font-size:var(--ui-font-caption);line-height:1.6}
.appearance-theme-option{min-width:0;display:flex;flex-direction:column;gap:8px;padding:9px;border:1px solid var(--app-line);border-radius:10px;background:var(--app-surface);color:var(--app-muted);font-size:var(--ui-font-caption);text-align:center;cursor:pointer}
.appearance-theme-option>span{display:block;width:100%;height:50px;border:0;border-radius:7px;background:#f4eee2}
.appearance-theme-option.theme-sepia>span{background:#e5cfaa}
.appearance-theme-option.theme-light>span,.appearance-theme-option.theme-white>span{background:#fff;border:1px solid #e5e6ed}
.appearance-theme-option.theme-dark>span{background:#25272c}
.appearance-theme-option.theme-system>span{background:linear-gradient(135deg,#fff 50%,#25272c 50%)}
.appearance-theme-option:focus-visible{outline:2px solid var(--app-accent);outline-offset:3px}
.appearance-theme-option.selected{border-color:var(--app-accent);color:var(--app-accent-ink);background:var(--app-accent-soft);font-weight:750}
.appearance-theme-option.selected>span{box-shadow:0 0 0 1px #b9aef0}
.appearance-video-note{margin:8px 0;color:var(--app-subtle);font-size:var(--ui-font-caption);line-height:1.65}
.appearance-advanced{border-top:1px solid var(--app-line)}
.appearance-advanced>summary{display:flex;align-items:center;gap:9px;min-height:43px;color:var(--app-text);font-size:var(--ui-font-caption);font-weight:650;cursor:pointer;list-style:none}
.appearance-advanced>summary::-webkit-details-marker{display:none}
.appearance-advanced>summary span{margin-left:auto;color:var(--app-subtle);font-size:var(--ui-font-caption);font-weight:400}
.appearance-advanced>summary :deep(svg){width:16px;height:16px;transform:rotate(90deg)}
.appearance-advanced[open]>summary :deep(svg){transform:rotate(-90deg)}
.appearance-advanced .appearance-workspace-group:first-of-type{border-top:1px solid var(--app-line);padding-top:15px}
.appearance-error{margin:0;padding:10px 0;color:var(--app-danger);font-size:var(--ui-font-caption)}
@media(max-width:660px){
  .appearance-topbar{padding-inline:var(--ui-header-gutter)}
  .appearance-workspace{width:100%}
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

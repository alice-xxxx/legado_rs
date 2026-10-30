<script setup lang="ts">
import { Block as KBlock, Range as KRange, Button as KButton, SegmentedButton as KSegmentedButton } from "konsta/vue";
import KSegmented from "../../ui/SegmentedControl.vue";
// 本页直接修改全局响应式设置，由 settingsStore 自动保存。
import { computed, ref } from "vue";
import { pickReaderBackgroundImage } from "../../api/settings";
import type { ReaderSettings } from "../../api/types";
import { settings } from "./settingsStore";
import { isReadingMode, readingModes } from "./readerModes";
import { appThemes, defaultMaterialColor, materialColorPresets, readingThemes, resolvedMaterialColor, resolvedReadingTheme, setReaderTheme, uiStyles } from "./themeState";
import { backupBusy } from "./backupState";
import { shelfBatchRecoveryRequired } from "../../app/recoveryState";
import { notify } from "../../app/notifications";
import { routeNames } from "../../router";
import { returnToRoute } from "../../app/routeReturn";
import PrototypeIcon from "../../ui/PrototypeIcon.vue";
import SubpageHeader from "../../ui/SubpageHeader.vue";
import SelectMenu from "../../ui/SelectMenu.vue";

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

function changeReaderMode(mode: string): void {
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
    <SubpageHeader title="外观、阅读与播放" back-label="返回我的" @back="returnToSettings" />
    <div class="appearance-workspace ui-page-content">

      <main class="appearance-settings-stack">
      <KBlock strong inset class="appearance-workspace-group mx-0! my-0!">
        <header><h2>界面风格</h2></header>
        <KSegmented strong rounded class="mb-5" aria-label="界面风格">
          <KSegmentedButton v-for="style in uiStyles" :key="style[0]" type="button"
            :active="settings.uiStyle === style[0]" :aria-pressed="settings.uiStyle === style[0]"
            :disabled="blocked" @click="settings.uiStyle = style[0]">{{ style[1] }}</KSegmentedButton>
        </KSegmented>
        <template v-if="settings.uiStyle === 'material'">
          <header class="appearance-color-heading"><h2>主题颜色</h2>
            <KButton clear inline rounded type="button" :disabled="blocked || resolvedMaterialColor === defaultMaterialColor"
              @click="settings.materialColor = defaultMaterialColor">恢复默认</KButton>
          </header>
          <div class="appearance-color-options" role="group" aria-label="主题颜色">
            <KButton v-for="[color, name] in materialColorPresets" :key="color" clear rounded type="button"
              class="h-auto! min-h-16 min-w-0 flex-col gap-2 px-0! py-2!"
              :aria-label="name" :aria-pressed="resolvedMaterialColor === color" :disabled="blocked"
              @click="settings.materialColor = color">
              <span class="appearance-color-swatch" :style="{ background: color }"
                :class="{ 'is-selected': resolvedMaterialColor === color }"></span>{{ name }}
            </KButton>
          </div>
          <div class="appearance-custom-color">
            <label for="material-custom-color">自定义颜色</label>
            <input id="material-custom-color" type="color" :value="resolvedMaterialColor" :disabled="blocked"
              @input="settings.materialColor = ($event.target as HTMLInputElement).value" />
          </div>
        </template>
        <header><h2>应用主题</h2></header>
        <KSegmented strong rounded class="mb-5" aria-label="应用主题">
          <KSegmentedButton v-for="theme in appThemes" :key="theme[0]" type="button"
            :active="settings.theme === theme[0]" :aria-pressed="settings.theme === theme[0]"
            :disabled="blocked" @click="settings.theme = theme[0]">{{ theme[1] }}</KSegmentedButton>
        </KSegmented>
    <template v-if="settings.theme !== 'system'">
      <header class="reading-theme-heading"><h2>{{ settings.theme === 'light' ? '白天' : '夜间' }}阅读主题</h2>
        <small>切换应用主题时，同时使用这里保存的阅读配色。</small></header>
      <div class="appearance-theme-options">
        <KButton v-for="theme in readingThemes" :key="theme[0]" type="button"
          class="appearance-theme-option h-auto! flex-col! gap-2 rounded-[var(--ui-radius)]! px-2! py-2!" :tonal="resolvedReadingTheme === theme[0]" :clear="resolvedReadingTheme !== theme[0]" :class="`theme-${theme[0]}`"
          :aria-pressed="resolvedReadingTheme === theme[0]" :disabled="blocked" @click="setReaderTheme(theme[0])">
          <span></span>{{ theme[1] }}
        </KButton>
      </div>
    </template>
    <p v-else class="appearance-theme-note">随系统自动切换应用配色和对应的阅读主题。</p>
      </KBlock>
      <KBlock strong inset class="appearance-workspace-group mx-0! my-0!">
        <header><h2>文字阅读</h2></header>
    <div class="appearance-setting-row">
      <div class="appearance-setting-copy">
        <label for="reader-font-size">字体大小</label>
        <small>当前 {{ settings.reader.fontSizePx }} px</small>
      </div>
      <div class="appearance-range-control">
        <KRange
          input-id="reader-font-size"
          :min="12"
          :max="36"
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
      <KRange
        input-id="reader-line-height"
        class="appearance-single-range"
        :min="1.2"
        :max="2.8"
        :step="0.1"
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
      <SelectMenu id="reader-font-family" label="正文字体" v-model="settings.reader.fontFamily" :disabled="blocked"
        :options="[{ value: 'serif', label: '衬线字体' }, { value: 'sans', label: '无衬线字体' }, { value: 'system', label: '系统字体' }, { value: 'mono', label: '等宽字体' }]" />
    </div>


    <div class="appearance-setting-row">
      <div class="appearance-setting-copy">
        <label for="reader-turn-style">阅读方式</label>
        <small>分页切换效果或上下连续滚动</small>
      </div>
      <SelectMenu id="reader-turn-style" label="阅读方式" :model-value="settings.reader.readingMode ?? 'cover'"
        :disabled="blocked" :options="readingModes" @update:model-value="changeReaderMode" />
    </div>
      </KBlock>
      <KBlock strong inset class="appearance-workspace-group mx-0! my-0!">
        <header><h2>音频</h2></header>
    <div class="appearance-setting-row">
      <div class="appearance-setting-copy">
        <label for="reader-default-audio-speed">音频默认倍速</label>
        <small>播放音频章节时使用的默认速度</small>
      </div>
      <SelectMenu id="reader-default-audio-speed" label="音频默认倍速" v-model="settings.reader.audioPlaybackRate"
        :disabled="blocked" :options="playbackRates.map(value => ({ value, label: value + '×' }))" />
    </div>
    <div class="appearance-setting-row">
      <div class="appearance-setting-copy">
        <label for="reader-audio-skip">跳过时长</label>
        <small>音频播放页快进与后退按钮，系统媒体控制也使用此时长</small>
      </div>
      <SelectMenu id="reader-audio-skip" label="跳过时长" v-model="settings.reader.audioSkipSeconds"
        :disabled="blocked" :options="[10, 15, 30].map(value => ({ value, label: value + ' 秒' }))" />
    </div>
      </KBlock>
      <KBlock strong inset class="appearance-workspace-group mx-0! my-0!">
        <header><h2>视频</h2></header>
        <div class="appearance-setting-row">
          <div class="appearance-setting-copy">
            <label for="reader-default-video-speed">默认倍速</label>
            <small>打开新视频时使用；与音频倍速分开保存</small>
          </div>
          <SelectMenu id="reader-default-video-speed" label="视频默认倍速" v-model="settings.reader.videoPlaybackRate"
            :disabled="blocked" :options="playbackRates.map(value => ({ value, label: value + '×' }))" />
        </div>
        <p class="appearance-video-note">清晰度和字幕取决于内容来源及播放器实际提供的轨道；可用的清晰度在视频播放页面选择。</p>
      </KBlock>
      <details class="appearance-advanced">
        <summary>更多阅读选项 <span>预读与自定义背景</span><PrototypeIcon name="chevron-right"/></summary>
      <KBlock strong inset class="appearance-workspace-group mx-0! my-4!">
        <header><h2>预读</h2></header>
    <div class="appearance-setting-row">
      <div class="appearance-setting-copy">
        <label for="preload-count">预读章节</label>
        <small>提前准备 {{ settings.reader.preloadCount }} 章</small>
      </div>
      <div class="appearance-range-control">
        <KRange
          input-id="preload-count"
          :min="1"
          :max="20"
          :step="1"
          :value="settings.reader.preloadCount"
          :disabled="blocked"
          @input="changeSetting('preloadCount', Number(($event.target as HTMLInputElement).value))"
        />
      </div>
    </div>




      </KBlock>
      <KBlock strong inset class="appearance-workspace-group mx-0! my-4!">
        <header><h2>阅读背景</h2></header>
    <div class="appearance-setting-row">
      <div class="appearance-setting-copy">
        <label>阅读背景图片</label>
        <small>选择 JPEG、PNG 或 WebP 图片</small>
      </div>
      <div class="appearance-image-controls">
        <img v-if="settings.reader.backgroundImageSrc" :src="settings.reader.backgroundImageSrc"/>
        <span v-else class="appearance-unset">未设置</span>
        <KButton rounded inline tonal small
          type="button"
          class="gap-2"
          :disabled="backgroundBusy || blocked"
          @click="chooseBackground"
        >{{ backgroundBusy ? '处理中…' : settings.reader.backgroundImageSrc ? '更换图片' : '选择图片' }}</KButton>
        <KButton rounded inline clear
          v-if="settings.reader.backgroundImageSrc"
          type="button"
          class="gap-2"
          :disabled="backgroundBusy || blocked"
          @click="clearBackground"
        >清除</KButton>
      </div>
    </div>

    <div class="appearance-setting-row">
      <div class="appearance-setting-copy">
        <label for="reader-background-image-opacity">背景图片强度</label>
        <small>低强度能让正文保持清晰</small>
      </div>
      <div class="appearance-range-control">
        <KRange
          input-id="reader-background-image-opacity"
          :min="0"
          :max="100"
          :step="1"
          :value="settings.reader.backgroundImageOpacity"
          :disabled="backgroundBusy || blocked"
          @input="changeSetting('backgroundImageOpacity', Number(($event.target as HTMLInputElement).value))"
        />
        <strong>{{ settings.reader.backgroundImageOpacity }}%</strong>
      </div>
    </div>
    <p v-if="backgroundError" class="appearance-error">{{ backgroundError }}</p>


      </KBlock>

      </details>

      </main>
    </div>
  </section>
</template>

<style scoped>
@layer components {
.appearance-settings-page{width:100%;min-width:0;color:var(--app-text)}

.appearance-settings-stack{display:grid;gap:var(--ui-space-6);min-width:0}
.appearance-workspace-group>header{margin:0 0 8px}
.appearance-workspace-group h2{margin:0;color:var(--app-text);font-size:var(--ui-font-body);font-weight:600}
.appearance-color-heading{display:flex;align-items:center;justify-content:space-between;gap:8px}
.appearance-color-options{display:grid;grid-template-columns:repeat(6,minmax(0,1fr));gap:4px}
.appearance-color-swatch{width:28px;height:28px;border-radius:50%;border:1px solid var(--app-line)}
.appearance-color-swatch.is-selected{outline:2px solid var(--app-accent);outline-offset:3px}
.appearance-custom-color{display:flex;align-items:center;justify-content:space-between;gap:12px;min-height:56px;margin-bottom:16px}
.appearance-custom-color input{width:48px;height:44px;padding:4px;border:1px solid var(--app-line);border-radius:var(--ui-radius);background:var(--app-surface);cursor:pointer}
.appearance-custom-color input:focus-visible{outline:2px solid var(--app-accent);outline-offset:3px}
.appearance-setting-row{box-sizing:border-box;display:grid;grid-template-columns:minmax(0,1fr) minmax(140px,.8fr);align-items:center;gap:10px;min-height:58px;padding:9px 0;border-bottom:1px solid var(--app-line)}
.appearance-workspace-group .appearance-setting-row:last-child{border-bottom:0}
.appearance-setting-copy{display:flex;flex-direction:column;gap:2px;min-width:0}
.appearance-setting-copy label{font-size:var(--ui-font-body);font-weight:500;color:var(--app-text)}
.appearance-setting-copy small{font-size:var(--ui-font-caption);line-height:1.45;color:var(--app-subtle)}
.appearance-range-control{display:flex;align-items:center;gap:8px;min-width:0}
.appearance-range-control input, .appearance-single-range{width:100%;min-width:0;accent-color:var(--app-accent)}
.appearance-range-control strong{min-width:38px;text-align:right;color:var(--app-accent-ink);font-size:var(--ui-font-caption);font-variant-numeric:tabular-nums}
.appearance-image-controls{display:flex;align-items:center;justify-content:flex-end;flex-wrap:wrap;gap:8px}
.appearance-image-controls img{width:42px;height:42px;object-fit:cover;border-radius:8px}
.appearance-unset{font-size:var(--ui-font-caption);color:var(--app-subtle)}
.appearance-theme-options{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:7px;width:100%}
.reading-theme-heading{margin:16px 0 8px}
.reading-theme-heading small, .appearance-theme-note{color:var(--app-muted);font-size:var(--ui-font-caption);line-height:1.6}
.appearance-theme-option>span{display:block;width:100%;height:50px;border:0;border-radius:7px;background:#f4eee2}
.appearance-theme-option.theme-sepia>span{background:#e5cfaa}
.appearance-theme-option.theme-white>span{background:#fff;border:1px solid #e5e6ed}
.appearance-theme-option.theme-dark>span{background:#25272c}
.appearance-video-note{margin:8px 0;color:var(--app-subtle);font-size:var(--ui-font-caption);line-height:1.65}
.appearance-advanced>summary{display:flex;align-items:center;gap:9px;min-height:43px;color:var(--app-text);font-size:var(--ui-font-caption);font-weight:500;cursor:pointer;list-style:none}
.appearance-advanced>summary::-webkit-details-marker{display:none}
.appearance-advanced>summary span{margin-left:auto;color:var(--app-subtle);font-size:var(--ui-font-caption);font-weight:400}
.appearance-advanced>summary :deep(svg){width:16px;height:16px;transform:rotate(90deg)}
.appearance-advanced[open]>summary :deep(svg){transform:rotate(-90deg)}
.appearance-error{margin:0;padding:10px 0;color:var(--app-danger);font-size:var(--ui-font-caption)}
@media(max-width:660px){

  .appearance-setting-row{grid-template-columns:minmax(0,1fr) minmax(110px,.8fr);gap:8px}
  .appearance-theme-options{gap:5px}
  .appearance-theme-option>span{height:40px}
}
}
</style>

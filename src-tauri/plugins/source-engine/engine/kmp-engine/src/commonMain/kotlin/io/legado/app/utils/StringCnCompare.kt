package io.legado.app.utils

/**
 * 中文字符串排序 (拼音序), 由 app 端 `StringExtensions.cnCompare` 下沉。
 *
 * app 端原实现按 `Build.VERSION.SDK_INT >= N` 分支取 android.icu / java.text Collator,
 * minSdk 26 下 else 分支恒不可达, 故 androidMain actual 只保留 android.icu 分支。
 *
 * - androidMain: `android.icu.text.Collator(SIMPLIFIED_CHINESE)`
 * - jvmMain: `java.text.Collator(Locale.CHINA)`
 * - nativeMain: 使用公共拼音后备；同音字及表外字符保留既有的码点回退。
 */
expect fun String.cnCompare(other: String): Int

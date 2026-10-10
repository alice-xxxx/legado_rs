package com.script.quickjs

// 平台实现留在各自 source set；原平台的 expect 声明不进入解析器 commonMain。

import android.util.Log

/**
 * Android 平台 actual 实现: 复用 android.util.Log + System.loadLibrary。
 *
 * 不改变 Android 端现有行为, 与改造前完全等价。
 */
fun logQuickJsError(tag: String, msg: String, e: Throwable? = null) {
    Log.e(tag, msg, e)
}

fun logQuickJsWarn(tag: String, msg: String, e: Throwable? = null) {
    Log.w(tag, msg, e)
}

fun loadLegadoQuickJsNative() {
    System.loadLibrary("legado_quickjs")
}


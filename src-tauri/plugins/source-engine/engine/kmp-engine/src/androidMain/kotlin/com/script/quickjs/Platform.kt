package com.script.quickjs

import android.util.Log

internal actual fun logQuickJsError(tag: String, msg: String, e: Throwable?) { Log.e(tag, msg, e) }
internal actual fun logQuickJsWarn(tag: String, msg: String, e: Throwable?) { Log.w(tag, msg, e) }
internal actual fun loadLegadoQuickJsNative() { System.loadLibrary("legado_quickjs") }

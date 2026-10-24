package com.script.quickjs

internal expect fun logQuickJsError(tag: String, msg: String, e: Throwable? = null)
internal expect fun logQuickJsWarn(tag: String, msg: String, e: Throwable? = null)
internal expect fun loadLegadoQuickJsNative()

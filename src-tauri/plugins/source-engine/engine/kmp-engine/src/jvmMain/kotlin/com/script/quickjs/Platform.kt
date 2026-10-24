package com.script.quickjs

internal actual fun logQuickJsError(tag: String, msg: String, e: Throwable?) {
    System.err.println("$tag: $msg")
    e?.printStackTrace()
}
internal actual fun logQuickJsWarn(tag: String, msg: String, e: Throwable?) = logQuickJsError(tag, msg, e)

internal actual fun loadLegadoQuickJsNative() {
    System.load(requireNotNull(System.getProperty("legado.quickjs.lib")) { "QuickJS library path is not configured" })
}

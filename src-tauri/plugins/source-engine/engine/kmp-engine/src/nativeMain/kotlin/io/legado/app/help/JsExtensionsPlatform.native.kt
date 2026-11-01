package io.legado.app.help

internal expect fun isMainThreadPlatform(): Boolean

internal actual object JsPlatformRuntime {
    actual fun formatTimeUtc(time: Long, format: String, shiftHours: Int): String =
        formatSimpleUtcTime(time, format, shiftHours)
    actual fun isMainThread(): Boolean = isMainThreadPlatform()
    actual fun unsafeSslContext(): Any? = null
}

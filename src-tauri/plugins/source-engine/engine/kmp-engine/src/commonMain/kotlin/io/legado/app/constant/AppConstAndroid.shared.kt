package io.legado.app.constant

import io.legado.app.help.source.SourceCacheProviders
import io.legado.app.utils.randomUUIDString
import kotlinx.atomicfu.locks.SynchronizedObject
import kotlinx.atomicfu.locks.synchronized
import kotlin.concurrent.Volatile

/** Compatibility for scripts using java.androidId(), without depending on app startup. */
object AndroidIdHolder {
    @Volatile var value: String = "null"
    internal val lock = SynchronizedObject()
}

val AppConst.androidId: String get() {
    if (AndroidIdHolder.value.length >= 16) return AndroidIdHolder.value
    val cache = SourceCacheProviders.impl ?: return AndroidIdHolder.value
    return synchronized(AndroidIdHolder.lock) {
        cache.get("source-engine-device-id")?.takeIf { it.length >= 16 }
            ?: randomUUIDString().replace("-", "").take(16).also { cache.put("source-engine-device-id", it) }
    }
}

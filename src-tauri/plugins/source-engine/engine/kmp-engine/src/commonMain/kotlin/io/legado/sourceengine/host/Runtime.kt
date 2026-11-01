package io.legado.sourceengine.host

import io.legado.app.model.SharedJsScope
import io.legado.app.model.script.JsBindingInjector
import io.legado.app.model.script.SharedLibraryScopes
import io.legado.app.utils.DeadlineRegexReplacer
import io.legado.app.utils.RegexReplacers
import kotlinx.atomicfu.locks.SynchronizedObject
import kotlinx.atomicfu.locks.synchronized

internal object HostRuntime {
    private val lock = SynchronizedObject()
    private var installed = false

    fun install(image: (String) -> String) = synchronized(lock) {
        if (installed) return@synchronized
        installFiles()
        installStores()
        JsBindingInjector.registerImageOps(NativeImages(image))
        SharedJsScope.registerProviders { SharedLibraryScopes }
        RegexReplacers.register(DeadlineRegexReplacer)
        installPlatformRuntime()
        installed = true
    }
}

internal expect fun installPlatformRuntime()

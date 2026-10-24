package io.legado.sourceengine.bridge

import io.legado.app.model.script.registerNativeJsEngines
import io.legado.app.utils.NativeRegexReplacer
import io.legado.app.utils.RegexReplacers

object IosSourceEngineRuntime {
    private val installOnce = lazy(LazyThreadSafetyMode.SYNCHRONIZED) {

        io.legado.sourceengine.bridge.installHostServices()

        RegexReplacers.register(NativeRegexReplacer)
        registerNativeJsEngines(RustImageOpsProvider(::rustImageRequest))
        io.legado.app.help.http.registerNativeHttpProvider()
    }

    fun install() {
        installOnce.value
    }
}

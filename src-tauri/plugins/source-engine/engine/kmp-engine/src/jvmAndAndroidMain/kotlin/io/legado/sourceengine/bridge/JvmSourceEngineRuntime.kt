package io.legado.sourceengine.bridge

import io.legado.app.help.JsCryptoProviderJvm
import io.legado.app.help.JsCryptoProviders
import io.legado.app.model.SharedJsScope
import io.legado.app.model.script.JsBindingInjector
import io.legado.app.model.script.JsEngineType
import io.legado.app.model.script.JsEngines
import io.legado.app.model.script.quickjs.QuickJsJsEngine
import io.legado.app.model.script.quickjs.QuickJsSharedJsScopeBase
import io.legado.app.utils.RegexReplacerImpl
import io.legado.app.utils.RegexReplacers
import java.util.concurrent.ConcurrentHashMap

internal object JvmSourceEngineRuntime {
    private var initialized = false

    @Synchronized
    fun install(imageRequest: (String) -> String) {
        if (initialized) return
        installHostServices()
        JsCryptoProviders.register(JsCryptoProviderJvm)
        RegexReplacers.register(RegexReplacerImpl)
        JsEngines.registerProvider { type ->
            check(type == JsEngineType.QUICKJS)
            QuickJsJsEngine
        }
        SharedJsScope.registerProviders { type ->
            check(type == JsEngineType.QUICKJS)
            HostJsScope
        }
        JsBindingInjector.registerImageOps(RustImageOpsProvider(imageRequest))
        initialized = true
    }
}

private object HostJsScope : QuickJsSharedJsScopeBase() {
    private val libraries = ConcurrentHashMap<String, String>()
    override fun cachedJsLibContent(name: String): String? = libraries[name]
    override fun storeJsLibContent(name: String, content: String) { libraries[name] = content }
    override fun downloadJsLibContent(url: String): String = SourceEngineHostRpc.downloadText(url)
    override fun jsLibDownloadFailedException(url: String): Exception =
        IllegalStateException("Failed to download source jsLib: $url")
}

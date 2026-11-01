package io.legado.sourceengine.host

import io.legado.app.help.JsCryptoProviderJvm
import io.legado.app.help.JsCryptoProviders
import io.legado.app.model.script.JsEngines
import io.legado.app.model.script.quickjs.QuickJsJsEngine

internal actual fun installPlatformRuntime() {
    HostHttpClient.install()
    JsCryptoProviders.register(JsCryptoProviderJvm)
    JsEngines.registerProvider { QuickJsJsEngine }
}

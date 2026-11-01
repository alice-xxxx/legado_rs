package io.legado.sourceengine.host

import io.legado.app.help.JsCryptoProviderNative
import io.legado.app.help.JsCryptoProviders
import io.legado.app.model.script.JsEngines
import io.legado.app.model.script.NativeJsEngine

internal actual fun installPlatformRuntime() {
    HostHttpClient.install()
    JsCryptoProviders.register(JsCryptoProviderNative)
    JsEngines.registerProvider { NativeJsEngine }
}

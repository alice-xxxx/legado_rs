package io.legado.sourceengine.host

import io.legado.app.help.http.*
import io.legado.app.model.script.registerNativeJsEngines
import io.legado.app.utils.*
import kotlin.concurrent.Volatile

internal object HostRuntime{
    @Volatile private var installed=false
    fun install(){
        if(installed)return
        installFiles();installStores();registerNativeJsEngines(NativeImages(::image));RegexReplacers.register(DeadlineRegexReplacer)
        HostHttpClient.install()
        installed=true
    }
}

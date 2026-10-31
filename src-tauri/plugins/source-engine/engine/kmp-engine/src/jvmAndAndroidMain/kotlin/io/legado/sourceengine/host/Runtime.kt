package io.legado.sourceengine.host

import io.legado.app.help.*
import io.legado.app.model.SharedJsScope
import io.legado.app.model.script.*
import io.legado.app.model.script.quickjs.*
import io.legado.app.utils.*

/** JVM 与 Android 使用同一套脚本和宿主能力初始化。 */
internal object HostRuntime {
    private var installed=false
    @Synchronized fun install(image:(String)->String){
        if(installed)return
        installFiles();installStores();HostHttpClient.install();JsCryptoProviders.register(JsCryptoProviderJvm);RegexReplacers.register(DeadlineRegexReplacer)
        JsEngines.registerProvider{type->check(type==JsEngineType.QUICKJS);QuickJsJsEngine}
        SharedJsScope.registerProviders{type->check(type==JsEngineType.QUICKJS);SharedLibraryScopes}
        JsBindingInjector.registerImageOps(NativeImages(image));installed=true
    }
}

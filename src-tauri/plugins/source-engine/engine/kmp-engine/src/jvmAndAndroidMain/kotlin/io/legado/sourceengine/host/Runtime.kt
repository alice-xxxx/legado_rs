package io.legado.sourceengine.host

import io.legado.app.help.*
import io.legado.app.model.SharedJsScope
import io.legado.app.model.script.*
import io.legado.app.model.script.quickjs.*
import io.legado.app.utils.*
import kotlinx.coroutines.runBlocking
import java.util.concurrent.ConcurrentHashMap

/** JVM 与 Android 使用同一套脚本和宿主能力初始化。 */
internal object HostRuntime {
    private var installed=false
    @Synchronized fun install(image:(String)->String){
        if(installed)return
        installFiles();installStores();HostHttpClient.install();JsCryptoProviders.register(JsCryptoProviderJvm);RegexReplacers.register(RegexReplacerImpl)
        JsEngines.registerProvider{type->check(type==JsEngineType.QUICKJS);QuickJsJsEngine}
        SharedJsScope.registerProviders{type->check(type==JsEngineType.QUICKJS);Libraries}
        JsBindingInjector.registerImageOps(NativeImages(image));installed=true
    }
}
private object Libraries:QuickJsSharedJsScopeBase(){
    private val cache=ConcurrentHashMap<String,String>()
    override fun cachedJsLibContent(name:String)=cache[name]
    override fun storeJsLibContent(name:String,content:String){cache[name]=content}
    override fun downloadJsLibContent(url:String)=runBlocking{requireNotNull(SourceEngineHostRegistry.current()).executeHttp(HostHttpRequest(url)).body.decodeToString()}
    override fun jsLibDownloadFailedException(url:String)=IllegalStateException("脚本库下载失败：$url")
}

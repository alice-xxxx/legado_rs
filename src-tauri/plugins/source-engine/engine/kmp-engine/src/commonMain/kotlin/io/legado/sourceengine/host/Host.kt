package io.legado.sourceengine.host

import io.legado.app.help.coroutine.IoDispatcher
import kotlinx.coroutines.*
import kotlin.coroutines.*

interface SourceEngineHost {
    val executionId:String
    val dataDirectory:String
    val supportsDynamicBrowser:Boolean get()=true
    val storage:HostStorage
    suspend fun executeHttp(request:HostHttpRequest):HostHttpResponse
    suspend fun executeBrowser(request:HostBrowserRequest):HostBrowserResponse
}
internal class SourceEngineHostContext(val host:SourceEngineHost):AbstractCoroutineContextElement(Key) {
    companion object Key:CoroutineContext.Key<SourceEngineHostContext>
}
/** 只保护脚本线程状态；HTTP 挂起时释放通道。 */
object SourceEngineHostRegistry {
    private val lane=IoDispatcher.limitedParallelism(1)
    fun current():SourceEngineHost?=currentHost()
    @kotlin.jvm.JvmStatic fun executionCancelled()=current()?.executionId?.let(::sourceExecutionCancelled)?:false
    internal fun <T> onThread(host:SourceEngineHost,block:()->T):T {
        val old=currentHost();setHost(host);return try{block()}finally{setHost(old)}
    }
    internal suspend fun <T> withHost(host:NativeHost,block:suspend()->T):T=withContext(object:CoroutineDispatcher(){
        override fun dispatch(context:CoroutineContext,block:Runnable){lane.dispatch(context,Runnable{onThread(host){block.run()}})}
    }+SourceEngineHostContext(host)){try{block()}finally{host.imageScope.close()}}
}
internal expect fun currentHost():SourceEngineHost?
internal expect fun setHost(host:SourceEngineHost?)
internal expect fun sourceExecutionCancelled(id:String):Boolean

/** 每项原生能力各自实现，不使用事件总线。 */
abstract class NativeHost(override val executionId:String,override val dataDirectory:String):SourceEngineHost {
    internal val imageScope=NativeImages.Scope()
    protected abstract fun http(request:String):String
    protected abstract fun store(request:String):String
    protected abstract suspend fun browser(request:String):String
    final override val storage:HostStorage=object:HostStorage {
        override suspend fun get(namespace:String,key:String)=storage(namespace,"get",key)
        override suspend fun put(namespace:String,key:String,value:String,ttlSeconds:Long?){storage(namespace,"put",key,value,ttlSeconds)}
        override suspend fun delete(namespace:String,key:String){storage(namespace,"delete",key)}
        override suspend fun deletePrefixes(namespace:String,prefixes:List<String>){storage(namespace,"deletePrefixes",prefixes=prefixes)}
        override suspend fun deleteContains(namespace:String,value:String){storage(namespace,"deleteContains",contains=value)}
        override suspend fun clear(namespace:String){storage(namespace,"clear")}
        override suspend fun hasCookieJarCookies(scope:String,sourceUrl:String)=storage("cookies","jarHasCookies",scope,sourceUrl)=="true"
    }
    private fun storage(namespace:String,operation:String,key:String?=null,value:String?=null,ttl:Long?=null,prefixes:List<String>?=null,contains:String?=null):String?=
        StorageWire.decode(store(StorageWire.encode(executionId,namespace,operation,key,value,ttl,prefixes,contains)))
    final override suspend fun executeHttp(request:HostHttpRequest)=withContext(Dispatchers.Default){HttpWire.decode(http(HttpWire.encode(request.copy(executionId=executionId))))}
    final override suspend fun executeBrowser(request:HostBrowserRequest)=BrowserWire.decode(browser(BrowserWire.encode(request.copy(executionId=executionId))),request)
}

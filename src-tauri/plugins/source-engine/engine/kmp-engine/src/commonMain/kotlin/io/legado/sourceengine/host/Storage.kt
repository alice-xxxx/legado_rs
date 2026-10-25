package io.legado.sourceengine.host

import kotlinx.coroutines.runBlocking
import io.legado.app.help.source.SourceStorage
import io.legado.app.help.source.SourceStorageNamespace
import kotlinx.serialization.json.*

interface HostStorage {
    suspend fun get(namespace:String,key:String):String?
    suspend fun put(namespace:String,key:String,value:String,ttlSeconds:Long?=null)
    suspend fun delete(namespace:String,key:String)
    suspend fun deletePrefixes(namespace:String,prefixes:List<String>)
    suspend fun deleteContains(namespace:String,value:String)
    suspend fun clear(namespace:String)
    suspend fun hasCookieJarCookies(scope:String,sourceUrl:String):Boolean
}
internal object StorageWire {
    fun encode(id:String,namespace:String,operation:String,key:String?,value:String?,ttl:Long?,prefixes:List<String>?,contains:String?)=buildJsonObject {
        put("executionId",id);put("namespace",namespace);put("operation",operation);put("key",key);put("value",value);put("ttlSeconds",ttl);put("contains",contains)
        prefixes?.let{put("keyPrefixes",JsonArray(it.map(::JsonPrimitive)))}
    }.toString()
    fun decode(text:String):String?=payload(text)["value"]?.takeUnless{it==JsonNull}?.jsonPrimitive?.content
}
/** 脚本的同步缓存调用只访问当前宿主端口。 */
internal object EngineStorage:SourceStorage {
    private fun <T> use(block:suspend HostStorage.()->T):T=runBlocking{requireNotNull(SourceEngineHostRegistry.current()).storage.block()}
    override fun get(namespace:SourceStorageNamespace,key:String)=use{get(namespace.value,key)}
    override fun put(namespace:SourceStorageNamespace,key:String,value:String,ttlSeconds:Long)=use{put(namespace.value,key,value,ttlSeconds)}
    override fun delete(namespace:SourceStorageNamespace,key:String)=use{delete(namespace.value,key)}
    override fun deletePrefixes(namespace:SourceStorageNamespace,prefixes:List<String>)=use{deletePrefixes(namespace.value,prefixes)}
    override fun deleteContains(namespace:SourceStorageNamespace,value:String)=use{deleteContains(namespace.value,value)}
    override fun clear(namespace:SourceStorageNamespace)=use{clear(namespace.value)}
}

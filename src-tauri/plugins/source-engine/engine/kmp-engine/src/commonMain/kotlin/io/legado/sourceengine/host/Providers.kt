package io.legado.sourceengine.host

import io.legado.app.constant.AppConst
import io.legado.app.help.*
import io.legado.app.help.http.*
import io.legado.app.help.source.*
import io.legado.app.help.toast.*
import kotlin.io.encoding.Base64

/** 规则层的旧协议对象委托给当前宿主能力，不注册应用服务或空实现。 */
internal fun installStores(){
    BackstageWebViewProviders.register(HostBackstageWebViews)
    VerificationUiProviders.register(HostVerificationUi)
    SourceStorageProviders.impl=EngineStorage
    Toasters.register(object:Toaster{
        override fun toast(message:String){println("[source] $message")}
        override fun toastLong(message:String)=toast(message)
    })
    CookieStoreProviders.register(SharedCookieStore);registerSharedCookieJarBridge()
    UserAgentProviders.impl=UserAgentProvider{AppConst.DEFAULT_USER_AGENT}
    SourceCacheProviders.impl=object:SourceCacheProvider {
        override fun get(key:String)=CacheManager.get(key)
        override fun put(key:String,value:String)=CacheManager.put(key,value)
        override fun put(key:String,value:String,saveTime:Int)=CacheManager.put(key,value,saveTime)
        override fun delete(key:String)=CacheManager.delete(key)
        override fun getFromMemory(key:String)=CacheManager.getFromMemory(key)
        override fun putMemory(key:String,value:Any)=CacheManager.putMemory(key,value)
        override fun deleteMemory(key:String)=CacheManager.deleteMemory(key)
        override fun clearMemoryByPrefixes(prefixes:List<String>)=CacheManager.clearMemoryByPrefixes(prefixes)
        override fun asBinding():Any=CacheManager
    }
    FileCacheProviders.impl=object:FileCacheProvider {
        private fun namespace(persistent:Boolean)=if(persistent)SourceStorageNamespace.PersistentFileCache else SourceStorageNamespace.FileCache
        override fun put(key:String,value:String,saveTime:Int,persistent:Boolean)=EngineStorage.put(namespace(persistent),key,value,saveTime.toLong())
        override fun getAsString(key:String,persistent:Boolean)=EngineStorage.get(namespace(persistent),key)
        override fun put(key:String,value:ByteArray,saveTime:Int,persistent:Boolean)=put(key,"base64:"+Base64.encode(value),saveTime,persistent)
        override fun getAsBinary(key:String,persistent:Boolean)=getAsString(key,persistent)?.takeIf{it.startsWith("base64:")}?.removePrefix("base64:")?.let{Base64.decode(it)}
        override fun remove(key:String,persistent:Boolean)=EngineStorage.delete(namespace(persistent),key)
    }
    ExploreKindsCacheProviders.impl=object:ExploreKindsCacheProvider{
        override fun getAsString(key:String)=EngineStorage.get(SourceStorageNamespace.ExploreKinds,key)
        override fun put(key:String,value:String)=EngineStorage.put(SourceStorageNamespace.ExploreKinds,key,value)
        override fun remove(key:String)=EngineStorage.delete(SourceStorageNamespace.ExploreKinds,key)
    }
    RuleBigDataProviders.impl=object:RuleBigDataProvider{
        private fun key(vararg parts:String)=parts.joinToString(":"){Base64.UrlSafe.encode(it.encodeToByteArray()).trimEnd('=')}
        private fun put(key:String,value:String?){if(value==null)EngineStorage.delete(SourceStorageNamespace.BookVariables,key)else EngineStorage.put(SourceStorageNamespace.BookVariables,key,value)}
        override fun putBookVariable(bookUrl:String,key:String,value:String?)=put(key(bookUrl,key),value)
        override fun getBookVariable(bookUrl:String,key:String?)=EngineStorage.get(SourceStorageNamespace.BookVariables,key(bookUrl,key.orEmpty()))
        override fun hasBookVariable(bookUrl:String,key:String)=getBookVariable(bookUrl,key)!=null
        override fun putChapterVariable(bookUrl:String,chapterUrl:String,key:String,value:String?)=put(key(bookUrl,chapterUrl,key),value)
        override fun getChapterVariable(bookUrl:String,chapterUrl:String,key:String)=EngineStorage.get(SourceStorageNamespace.BookVariables,key(bookUrl,chapterUrl,key))
    }
    SourceNetworkProviders.impl=object:SourceNetworkProvider{
        override fun getCookie(tag:String)=SharedCookieStore.getCookie(tag)
        override fun replaceCookie(tag:String,cookie:String)=SharedCookieStore.replaceCookie(tag,cookie)
        override fun removeCookie(tag:String)=SharedCookieStore.removeCookie(tag)
        override fun asBinding():Any=SharedCookieStore
    }
    SourceDebugLoggers.impl=object:SourceDebugLogger{
        override fun log(key:String,msg:String,print:Boolean,state:Int,showTime:Boolean){if(print)println("[source:$state] $key "+msg.take(4096))}
        override fun log(msg:String)=log("",msg)
    }
}

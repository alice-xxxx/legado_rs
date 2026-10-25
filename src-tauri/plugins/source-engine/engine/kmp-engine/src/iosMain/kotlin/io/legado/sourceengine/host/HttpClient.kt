package io.legado.sourceengine.host

import io.legado.app.help.http.*
import kotlinx.atomicfu.locks.SynchronizedObject
import kotlinx.atomicfu.locks.synchronized

/** iOS 只缓存请求配置，HTTP 与代理都交给同一宿主能力。 */
internal object HostHttpClient:OkHttpClientProvider,OkHttpProxyClientProvider{
    private val lock=SynchronizedObject()
    private val proxies=mutableMapOf<String,KmpHttpClient>()
    override val okHttpClient=KmpHttpClientBuilder().build()
    override fun getProxyClient(proxy:String?):KmpHttpClient{
        val url=hostProxyUrl(proxy)?:return okHttpClient
        return synchronized(lock){proxies.getOrPut(url){KmpHttpClientBuilder().proxy(url).build()}}
    }
    fun install(){OkHttpClientProviders.register(this);OkHttpProxyClientProviders.impl=this}
}

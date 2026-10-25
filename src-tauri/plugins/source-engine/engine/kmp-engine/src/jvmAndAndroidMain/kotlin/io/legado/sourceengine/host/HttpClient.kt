package io.legado.sourceengine.host

import io.legado.app.constant.AppConst
import io.legado.app.help.UserAgentProviders
import io.legado.app.help.http.*
import io.legado.app.utils.NetworkUtils
import kotlinx.coroutines.runBlocking
import okhttp3.*
import okhttp3.MediaType.Companion.toMediaTypeOrNull
import okhttp3.ResponseBody.Companion.toResponseBody
import okio.Buffer
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit

/** OkHttp 只构造规则请求和响应；不注册另一套网络引擎或应用下载监听。 */
internal object HostHttpClient:OkHttpClientProvider,OkHttpProxyClientProvider {
    private val proxies=ConcurrentHashMap<String,KmpHttpClient>()
    override val okHttpClient by lazy{client(null)}
    override fun getProxyClient(proxy:String?)=if(proxy.isNullOrBlank())okHttpClient else proxies.getOrPut(proxy){client(proxy)}
    fun install(){OkHttpClientProviders.register(this);OkHttpProxyClientProviders.impl=this}
    private data class Metadata(val cookies:Boolean)
    private fun client(proxy:String?):KmpHttpClient {
        return OkHttpClient.Builder()
            .connectTimeout(15,TimeUnit.SECONDS).writeTimeout(15,TimeUnit.SECONDS)
            .readTimeout(15,TimeUnit.SECONDS).callTimeout(15,TimeUnit.SECONDS)
            .addInterceptor{chain->
                val original=chain.request()
                val host=original.tag(SourceEngineHost::class.java)?:SourceEngineHostRegistry.current()?:error("缺少请求宿主")
                SourceEngineHostRegistry.onThread(host){
                    val cookies=original.header(cookieJarHeader)!=null
                    val builder=original.newBuilder().removeHeader(cookieJarHeader).tag(Metadata::class.java,Metadata(cookies))
                    when(original.header(AppConst.UA_NAME)){
                        null->builder.header(AppConst.UA_NAME,UserAgentProviders.get())
                        "null"->builder.removeHeader(AppConst.UA_NAME)
                    }
                    builder.header("Cache-Control","no-cache")
                    val bridge=CookieJarBridgeHolder.get()
                    val request=if(cookies)bridge?.loadRequest(builder.build())?:builder.build() else builder.build()
                    chain.proceed(request).also{if(cookies)bridge?.saveResponse(it)}
                }
            }
            .addInterceptor(DecompressInterceptor)
            .addInterceptor{chain->
                val request=chain.request()
                val host=request.tag(SourceEngineHost::class.java)?:SourceEngineHostRegistry.current()?:error("缺少请求宿主")
                val body=request.body?.let{Buffer().also(it::writeTo).readByteArray()}
                val headers=request.headers.map{(name,value)->HostHeader(name,value)}.toMutableList()
                if(headers.none{it.name.equals("Content-Type",true)})request.body?.contentType()?.let{headers+=HostHeader("Content-Type",it.toString())}
                val cookies=request.tag(Metadata::class.java)?.cookies==true
                val response=runBlocking{host.executeHttp(HostHttpRequest(
                    url=request.url.toString(),method=request.method,headers=headers,body=body,
                    connectTimeoutMs=chain.connectTimeoutMillis().toLong(),readTimeoutMs=chain.readTimeoutMillis().toLong(),
                    callTimeoutMs=chain.call().timeout().timeoutNanos().takeIf{it>0}?.let{TimeUnit.NANOSECONDS.toMillis(it).coerceAtLeast(1)},
                    maxRedirects=20,useCookieJar=cookies,cookieScope=if(cookies)NetworkUtils.getSubDomain(request.url.toString())else null,
                    proxy=hostProxyUrl(proxy),acceptInvalidCerts=true))}
                var prior:Response?=null
                for(redirect in response.redirects){
                    prior=Response.Builder().request(Request.Builder().url(redirect.fromUrl).build())
                        .protocol(Protocol.HTTP_1_1).code(redirect.status).message("").header("Location",redirect.toUrl)
                        .apply{prior?.let(::priorResponse)}.build()
                }
                val resultHeaders=Headers.Builder().apply{response.headers.forEach{add(it.name,it.value)}}.build()
                Response.Builder().request(request.newBuilder().url(response.finalUrl).build())
                    .protocol(Protocol.HTTP_1_1).code(response.status).message(response.reason).headers(resultHeaders)
                    .body(response.body.toResponseBody(resultHeaders["Content-Type"]?.toMediaTypeOrNull()))
                    .apply{prior?.let(::priorResponse)}.build()
            }.build()
    }
}

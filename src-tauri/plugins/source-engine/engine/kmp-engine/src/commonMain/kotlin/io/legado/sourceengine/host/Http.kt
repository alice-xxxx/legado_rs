package io.legado.sourceengine.host

import kotlinx.serialization.*
import kotlinx.serialization.json.*
import kotlin.io.encoding.Base64

@Serializable data class HostHeader(val name:String,val value:String)
data class HostHttpMultipartPart(val name:String,val text:String?=null,val fileName:String?=null,val contentType:String?=null,val bytes:ByteArray?=null)
data class HostHttpRequest(
    val url:String,val method:String="GET",val headers:List<HostHeader> = emptyList(),val body:ByteArray?=null,
    val multipart:List<HostHttpMultipartPart> = emptyList(),val connectTimeoutMs:Long?=null,val readTimeoutMs:Long?=null,val callTimeoutMs:Long?=null,
    val maxRedirects:Int?=null,val useCookieJar:Boolean=false,val cookieScope:String?=null,val proxy:String?=null,val acceptInvalidCerts:Boolean=false,val executionId:String?=null,
)
@Serializable data class HostHttpRedirect(val fromUrl:String,val toUrl:String,val status:Int)
data class HostHttpResponse(val requestedUrl:String,val finalUrl:String,val status:Int,val reason:String,val redirects:List<HostHttpRedirect> = emptyList(),val headers:List<HostHeader> = emptyList(),val body:ByteArray=byteArrayOf())
internal val wire=Json{ignoreUnknownKeys=true;encodeDefaults=true;explicitNulls=false;coerceInputValues=true}
internal fun payload(text:String):JsonObject {
    val value=wire.parseToJsonElement(text).jsonObject
    check(value["ok"]?.jsonPrimitive?.boolean==true){value["error"]?.jsonPrimitive?.content?:"宿主调用失败"}
    return value.getValue("value").jsonObject
}
internal object HttpWire {
    fun encode(r:HostHttpRequest)=buildJsonObject {
        put("executionId",r.executionId);put("url",r.url);put("method",r.method);put("headers",wire.encodeToJsonElement(r.headers))
        r.body?.let{put("bodyBase64",Base64.encode(it))}
        put("multipart",buildJsonArray{r.multipart.forEach{p->add(buildJsonObject{put("name",p.name);put("text",p.text);put("fileName",p.fileName);put("contentType",p.contentType);p.bytes?.let{put("bytesBase64",Base64.encode(it))}})}})
        put("connectTimeoutMs",r.connectTimeoutMs);put("readTimeoutMs",r.readTimeoutMs);put("callTimeoutMs",r.callTimeoutMs);put("maxRedirects",r.maxRedirects)
        put("useCookieJar",r.useCookieJar);put("cookieScope",r.cookieScope);put("proxy",r.proxy);put("acceptInvalidCerts",r.acceptInvalidCerts)
    }.toString()
    fun decode(text:String):HostHttpResponse {
        val r=payload(text)
        return HostHttpResponse(r.getValue("requestedUrl").jsonPrimitive.content,r.getValue("finalUrl").jsonPrimitive.content,r.getValue("status").jsonPrimitive.int,
            r["reason"]?.jsonPrimitive?.content.orEmpty(),wire.decodeFromJsonElement(r["redirects"]?:JsonArray(emptyList())),wire.decodeFromJsonElement(r.getValue("headers")),Base64.decode(r.getValue("bodyBase64").jsonPrimitive.content))
    }
}

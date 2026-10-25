package io.legado.sourceengine.host

import io.legado.app.utils.randomUUIDString
import kotlinx.serialization.*
import kotlinx.serialization.json.*

@Serializable enum class HostBrowserMode { OFFLINE_SNAPSHOT,DYNAMIC_PAGE,INTERACTIVE_PAGE }
@Serializable data class HostBrowserRequest(val capabilityId:String,val html:String,val baseUrl:String,val javaScript:String,val delayTimeMs:Long=0,val mode:HostBrowserMode=HostBrowserMode.OFFLINE_SNAPSHOT,val method:String="GET",val headers:List<HostHeader> = emptyList(),val useCookieJar:Boolean=false,val cookieScope:String?=null,val title:String="",val waitForResult:Boolean=true,val executionId:String?=null)
@Serializable data class HostBrowserResponse(val capabilityId:String,val finalUrl:String,val body:String,val cookieHeader:String="")
/** 内嵌页面作为内容传给浏览器，不作为登录网址或 Cookie 域名。 */
internal object BrowserContent {
    fun html(value:String):String? {
        val text=value.trim()
        if(text.startsWith("<"))return text
        if(!text.startsWith("data:text/html",true))return null
        val comma=text.indexOf(',')
        require(comma>=0){"HTML 数据地址无效"}
        val parts=text.substring(0,comma).split(';')
        require(parts.first().equals("data:text/html",true)){"HTML 数据类型无效"}
        val data=text.substring(comma+1)
        if(parts.any{it.equals("base64",true)})return io.legado.app.utils.EncoderUtils.base64Decode(data)
        val bytes=data.encodeToByteArray();val output=ByteArray(bytes.size);var read=0;var written=0
        while(read<bytes.size){
            if(bytes[read]==37.toByte()){
                require(read+2<bytes.size){"HTML 数据编码无效"}
                val high=bytes[read+1].toInt().toChar().digitToIntOrNull(16)
                val low=bytes[read+2].toInt().toChar().digitToIntOrNull(16)
                require(high!=null&&low!=null){"HTML 数据编码无效"}
                output[written++]=((high shl 4) or low).toByte();read+=3
            }else output[written++]=bytes[read++]
        }
        return output.copyOf(written).decodeToString()
    }
}
object SourceBrowserSnapshot { fun newCapabilityId()=randomUUIDString().replace("-","") }
internal object BrowserWire {
    fun encode(request:HostBrowserRequest)=wire.encodeToString(request)
    fun decode(text:String,request:HostBrowserRequest):HostBrowserResponse {
        val result=wire.decodeFromJsonElement<HostBrowserResponse>(payload(text))
        check(result.capabilityId==request.capabilityId){"浏览器结果不属于本次请求"}
        if(request.mode==HostBrowserMode.OFFLINE_SNAPSHOT)check(result.finalUrl==request.baseUrl){"离线快照地址被修改"}
        return result
    }
}

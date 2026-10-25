package io.legado.sourceengine.host

import io.legado.app.utils.randomUUIDString
import kotlinx.serialization.*
import kotlinx.serialization.json.*

@Serializable enum class HostBrowserMode { OFFLINE_SNAPSHOT,DYNAMIC_PAGE,INTERACTIVE_PAGE }
@Serializable data class HostBrowserRequest(val capabilityId:String,val html:String,val baseUrl:String,val javaScript:String,val delayTimeMs:Long=0,val mode:HostBrowserMode=HostBrowserMode.OFFLINE_SNAPSHOT,val method:String="GET",val headers:List<HostHeader> = emptyList(),val useCookieJar:Boolean=false,val cookieScope:String?=null,val title:String="",val waitForResult:Boolean=true,val executionId:String?=null)
@Serializable data class HostBrowserResponse(val capabilityId:String,val finalUrl:String,val body:String,val cookieHeader:String="")
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

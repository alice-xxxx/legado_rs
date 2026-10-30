package io.legado.app.help.source

import io.legado.app.data.entities.BaseSource
import io.legado.app.help.http.StrResponse
import kotlin.concurrent.Volatile

/** 源脚本的交互端口；宿主直接返回结果，核心不发送应用事件或轮询缓存。 */
interface VerificationUiProvider {
    fun awaitBrowser(source:BaseSource,url:String,title:String,refetchAfterSuccess:Boolean,html:String?=null):StrResponse
    fun getVerificationCode(url:String,source:BaseSource):String
    fun showLoginDialog(source:BaseSource)
    fun showSourceVariableDialog(source:BaseSource)
    fun startBrowser(source:BaseSource,url:String,title:String,saveResult:Boolean?,refetchAfterSuccess:Boolean?,asBottomSheet:Boolean=false,html:String?=null)
}
object VerificationUiProviders {
    @Volatile private var impl:VerificationUiProvider?=null
    fun register(provider:VerificationUiProvider?){impl=provider}
    fun current()=impl
    fun get():VerificationUiProvider=impl?:error("宿主未提供源交互能力")
}

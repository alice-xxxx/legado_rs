package io.legado.app.help.source

import io.legado.app.data.entities.BaseSource
import io.legado.app.exception.NoStackTraceException
import io.legado.app.help.JsExtensionsPlatform

/** 保留脚本验证语义，结果由宿主调用直接返回。 */
object SourceVerificationHelpShared {
    fun getVerificationResult(source:BaseSource?,url:String,title:String,useBrowser:Boolean,refetchAfterSuccess:Boolean=true):String {
        val owner=source?:throw NoStackTraceException("验证请求缺少书源")
        require(url.length<64*1024){"验证地址过长"}
        check(!JsExtensionsPlatform.isMainThread()){"验证请求必须在后台调用"}
        val provider=VerificationUiProviders.get()
        return if(useBrowser)provider.awaitBrowser(owner,url,title,refetchAfterSuccess).body?:throw NoStackTraceException("验证响应没有内容")
            else provider.getVerificationCode(url,owner).ifBlank{throw NoStackTraceException("验证结果为空")}
    }
    fun startBrowser(source:BaseSource?,url:String,title:String,saveResult:Boolean?=false,refetchAfterSuccess:Boolean?=true,asBottomSheet:Boolean=false,html:String?=null){
        val owner=source?:throw NoStackTraceException("浏览器请求缺少书源")
        require(url.length<64*1024){"浏览器地址过长"}
        VerificationUiProviders.get().startBrowser(owner,url,title,saveResult,refetchAfterSuccess,asBottomSheet,html)
    }
}

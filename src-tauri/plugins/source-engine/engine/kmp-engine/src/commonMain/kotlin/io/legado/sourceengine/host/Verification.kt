package io.legado.sourceengine.host

import io.legado.app.data.entities.BaseSource
import io.legado.app.help.coroutine.runBlockingInScope
import io.legado.app.help.http.CookieStoreProviders
import io.legado.app.help.http.StrResponse
import io.legado.app.help.http.cookieJarHeader
import io.legado.app.help.source.VerificationUiProvider
import io.legado.app.model.analyzeRule.AnalyzeUrlCore
import io.legado.app.utils.NetworkUtils
import kotlinx.serialization.encodeToString

/** 验证规则与登录按钮共用各平台已有的交互浏览器。 */
internal object HostVerificationUi : VerificationUiProvider {
    private fun render(source:BaseSource,url:String,title:String,html:String?,wait:Boolean,script:String=""):StrResponse {
        val host=SourceEngineHostRegistry.current()?:error("缺少浏览器宿主")
        val context=SourceEngineHostContext(host)
        return runBlockingInScope(context) {
            val inline=html?:BrowserContent.html(url)
            val targetUrl=if(inline!=null&&!url.startsWith("http"))source.getKey().takeIf{it.startsWith("http")}?:"https://foliver.invalid/" else url
            val target=AnalyzeUrlCore(targetUrl,source=source,coroutineContext=context)
            val body=inline?:if(target.isPost())target.getStrResponseAwait(allowWebView=false).body else null
            val response=host.executeBrowser(HostBrowserRequest(
                capabilityId=SourceBrowserSnapshot.newCapabilityId(),html=body.orEmpty(),baseUrl=target.url,
                javaScript=script,mode=HostBrowserMode.INTERACTIVE_PAGE,title=title,waitForResult=wait,
                headers=target.headerMap.entries.filterNot{it.key.equals(cookieJarHeader,true)}.map{HostHeader(it.key,it.value)},
                useCookieJar=source.enabledCookieJar==true,cookieScope=NetworkUtils.getSubDomain(target.url)
            ))
            if(response.cookieHeader.isNotBlank())CookieStoreProviders.get()!!.replaceCookie(response.finalUrl,response.cookieHeader)
            StrResponse(response.finalUrl,response.body)
        }
    }
    override fun awaitBrowser(source:BaseSource,url:String,title:String,refetchAfterSuccess:Boolean,html:String?):StrResponse {
        val response=render(source,url,title,html,true)
        if(!refetchAfterSuccess)return response
        val host=SourceEngineHostRegistry.current()?:error("缺少浏览器宿主")
        return AnalyzeUrlCore(if(html!=null)response.url else url,source=source,coroutineContext=SourceEngineHostContext(host)).getStrResponse(allowWebView=false)
    }
    override fun startBrowser(source:BaseSource,url:String,title:String,saveResult:Boolean?,refetchAfterSuccess:Boolean?,asBottomSheet:Boolean,html:String?) {
        render(source,url,title,html,false)
    }
    override fun getVerificationCode(url:String,source:BaseSource):String {
        val image=wire.encodeToString(url)
        val html="""<meta name="viewport" content="width=device-width,initial-scale=1"><p>请输入图片中的验证码，再点完成验证。</p><img id="captcha"><p><input id="code" autocomplete="off" aria-label="验证码"></p><script>document.getElementById('captcha').src=$image;</script>"""
        return render(source,url,"图片验证码",html,true,"document.getElementById('code').value").body.orEmpty()
    }
    override fun showLoginDialog(source:BaseSource) { error("此书源需要登录，请在书源管理中打开登录表单") }
    override fun showSourceVariableDialog(source:BaseSource) { error("请在书源编辑中设置源变量") }
}

package io.legado.sourceengine.api

import io.legado.app.data.entities.BookSource
import io.legado.app.data.entities.rule.RowUi
import io.legado.app.help.http.CookieStoreProviders
import io.legado.app.help.source.SourceCacheProviders
import io.legado.app.help.source.VerificationUiProvider
import io.legado.app.help.source.VerificationUiProviders
import io.legado.app.data.entities.BaseSource
import io.legado.app.help.http.StrResponse
import io.legado.app.help.coroutine.runBlockingInScope
import io.legado.app.model.analyzeRule.AnalyzeUrlCore
import io.legado.sourceengine.host.BrowserContent
import io.legado.sourceengine.host.HostBrowserRequest
import io.legado.sourceengine.host.HostBrowserMode
import io.legado.sourceengine.host.HostHeader
import io.legado.sourceengine.host.SourceBrowserSnapshot
import io.legado.sourceengine.host.SourceEngineHostContext
import io.legado.app.help.ui.*
import io.legado.app.help.toast.*
import io.legado.app.utils.KS_JSON
import io.legado.app.utils.NetworkUtils
import io.legado.sourceengine.host.SourceEngineHostRegistry
import kotlinx.coroutines.CancellationException
import kotlinx.serialization.encodeToString

/** 登录规则只产生数据和交互请求，原生窗口由平台插件处理。 */
object SourceLogin {
    private val fieldTypes=setOf("text","password","select","toggle")
    private fun isWeb(value:String)=value.startsWith("https://",true)||value.startsWith("http://",true)
    private fun webUrl(source:BookSource,raw:String):String? {
        val value=raw.trim().replace("{{source.bookSourceUrl}}",source.getKey())
        if(isWeb(value))return value
        val relative=!value.any(Char::isWhitespace)&&(value.startsWith("/")&&!value.startsWith("//")||value.startsWith("./")||value.startsWith("../")||
            Regex("^[\\w/-]+\\.(?:html?|php|aspx?)(?:[?#].*)?$",RegexOption.IGNORE_CASE).matches(value))
        return if(relative&&isWeb(source.getKey()))NetworkUtils.getAbsoluteURL(source.getKey(),value)else null
    }
    private fun label(source:BookSource,row:RowUi)=row.viewName?.let {
        runCatching{source.evalJS(source.getLoginJs().orEmpty()+"\n;"+it.removePrefix("@js:"))?.toString()}.getOrNull()
    }?.takeIf{it.isNotBlank()}?:row.name
    suspend fun login(r:LoginRequest):LoginState {
        val source=r.source;val url=webUrl(source,source.getLoginJs().orEmpty())
        val cache=SourceCacheProviders.impl!!;val key=source.getKey();val cookies=CookieStoreProviders.get()!!
        val webKey="loginWebUrl_$key"
        val cookieUrls=listOfNotNull(key.takeIf(::isWeb),url,cache.get(webKey)).distinct()
        var state=when(val action=r.action){
            LoginAction.Open->LoginState(form=form(source))
            LoginAction.Clear->{cookieUrls.forEach{cookies.removeCookieStrict(it)};cache.delete(webKey);source.removeLoginHeader();source.removeLoginInfo();LoginState(form=runCatching{form(source)}.getOrNull(),message="已清除登录数据。")}
            is LoginAction.ImportCookies->{
                val allowed=listOfNotNull(url,cache.get(webKey))+source.loginUi().orEmpty().mapNotNull{it.action?.let{value->webUrl(source,value)}}
                require(action.url in allowed){"Cookie 不属于当前登录地址"}
                require(action.cookies.length<=256*1024&&action.cookies.none{it=='\n'||it=='\r'||it=='\u0000'}&&action.cookies.split(';').size<=512){"Cookie 数据无效"}
                cookies.replaceCookie(action.url,action.cookies);LoginState(message="网页登录 Cookie 已保存。")
            }
            is LoginAction.Submit->if(url!=null)LoginState()else submit(source,action)
        }
        val saved=!cache.get("loginHeader_$key").isNullOrBlank()||!cache.get("userInfo_$key").isNullOrBlank()||cookieUrls.any{
            val domain=NetworkUtils.getSubDomain(it)
            cookies.getCookieNoSession(it).isNotBlank()||!cookies.getSessionCookie(domain).isNullOrBlank()
                ||SourceEngineHostRegistry.current()!!.storage.hasCookieJarCookies(domain,it)
        }
        if("login" in state.refreshTargets||r.action is LoginAction.Submit&&url==null)state=state.copy(form=form(source))
        val web=if(url!=null&&(r.action==LoginAction.Open||r.action is LoginAction.Submit))LoginWeb(url,cookies.getCookie(url))else state.web
        web?.let{cache.put(webKey,it.url)}
        return state.copy(hasLoginState=saved,web=web)
    }
    private fun form(source:BookSource):LoginForm {
        if(webUrl(source,source.getLoginJs().orEmpty())!=null)return LoginForm("web")
        val rows=source.loginUi().orEmpty();val saved=source.getLoginInfoMap().orEmpty()
        val fields=rows.filter{it.type in fieldTypes}.map{row->
            val choices=if(row.type=="select"||row.type=="toggle")row.chars.orEmpty()else emptyList()
            LoginField(row.name,label(source,row),row.type,row.type=="password",choices,
                if(row.type=="password")null else saved[row.name]?:row.defaultValue?:choices.firstOrNull()?:if(row.type=="toggle")"false"else "")
        }.distinctBy{it.name}
        require(fields.size<=64){"登录字段过多"}
        val actions=rows.mapIndexedNotNull{i,row->if(row.type==RowUi.Type.button&&!row.action.isNullOrBlank())LoginButton(i,label(source,row))else null}
        val canLogin=!source.getLoginJs().isNullOrBlank()
        require(canLogin||fields.isNotEmpty()||actions.isNotEmpty()){"此书源没有可用的登录表单。"}
        val layout=rows.mapIndexedNotNull{i,row->
            val kind=when{row.type in fieldTypes->"field";actions.any{it.id==i}->"action";row.type==RowUi.Type.title->"title";else->return@mapIndexedNotNull null}
            val basis=row.style?.layout_flexBasisPercent?:0F
            val cols=row.style?.cols?:when{!basis.isFinite()||basis<=0F->if(kind=="action")2 else 1;basis>=0.75F->1;basis>=0.42F->2;basis>=0.29F->3;else->4}
            LoginLayout(kind,row.name.takeIf{kind=="field"},i.takeIf{kind!="title"},label(source,row).takeIf{kind=="title"},cols.coerceIn(1,4),(row.style?.rows?:1).coerceIn(1,12))
        }
        return LoginForm("form",canLogin,fields,actions,layout)
    }
    private fun awaitBrowser(source:BaseSource,url:String,title:String,refetch:Boolean):StrResponse {
        val host=SourceEngineHostRegistry.current()?:error("缺少浏览器宿主")
        val context=SourceEngineHostContext(host)
        return runBlockingInScope(context) {
            val target=AnalyzeUrlCore(url,source=source,coroutineContext=context)
            val rendered=host.executeBrowser(HostBrowserRequest(
                capabilityId=SourceBrowserSnapshot.newCapabilityId(),html="",baseUrl=target.url,javaScript="",
                mode=HostBrowserMode.INTERACTIVE_PAGE,title=title,waitForResult=true,
                headers=target.headerMap.entries.map{HostHeader(it.key,it.value)},
                useCookieJar=source.enabledCookieJar==true,cookieScope=NetworkUtils.getSubDomain(target.url)
            ))
            if(rendered.cookieHeader.isNotBlank())CookieStoreProviders.get()!!.replaceCookie(target.url,rendered.cookieHeader)
            if(refetch)target.getStrResponse()else StrResponse(rendered.finalUrl,rendered.body)
        }
    }
    private fun submit(source:BookSource,action:LoginAction.Submit):LoginState {
        val form=form(source);require(action.fields.size<=64&&action.fields.all{(name,value)->name.isNotEmpty()&&name.length<=128&&value.length<=16384}){"登录字段无效"}
        val before=source.getLoginInfoMap().orEmpty()
        val fields=form.fields.associate{it.name to (action.fields[it.name]?.takeUnless{value->it.password&&value.isEmpty()}?:before[it.name]?:it.initialValue.orEmpty())}
        val rows=source.loginUi().orEmpty()
        val row=action.button?.let{rows.getOrNull(it)?:error("登录动作无效")}
        if(row!=null)require(form.layout.any{it.id==action.button}){"登录动作不可执行"}
        val input=row?.type in fieldTypes
        val scripts=mutableListOf<String>()
        // 点击按钮时一并落实尚未提交的字段动作，避免失焦与点击之间丢失最后一次输入。
        if(!input)rows.filter{it.type in fieldTypes&&before[it.name]!=fields[it.name]}.mapNotNullTo(scripts){it.action?.takeIf{it.isNotBlank()&&webUrl(source,it)==null}}
        val web=row?.action?.let{webUrl(source,it)}
        if(row!=null&&web==null)row.action?.takeIf{it.isNotBlank()}?.let(scripts::add)
        if(row==null)scripts+="if(typeof login==='function'){login.apply(this);}"
        val messages=mutableListOf<String>();val updates=mutableMapOf<String,String>();val refresh=linkedSetOf<String>();var copy:String?=null
        val oldUi=JsUiProviders.impl;val oldToast=Toasters.current()
        val oldVerification=VerificationUiProviders.current();var pendingWeb=web?.let{LoginWeb(it,CookieStoreProviders.get()!!.getCookie(it))}
        VerificationUiProviders.register(object:VerificationUiProvider {
            override fun startBrowser(source:BaseSource,url:String,title:String,saveResult:Boolean?,refetchAfterSuccess:Boolean?,asBottomSheet:Boolean) {
                val html=BrowserContent.html(url)
                if(html!=null){
                    val host=SourceEngineHostRegistry.current()?:error("缺少浏览器宿主")
                    runBlockingInScope(SourceEngineHostContext(host)){
                        host.executeBrowser(HostBrowserRequest(
                            capabilityId=SourceBrowserSnapshot.newCapabilityId(),html=html,
                            baseUrl=source.getKey().takeIf(::isWeb)?:"https://foliver.invalid/",
                            javaScript="",mode=HostBrowserMode.INTERACTIVE_PAGE,title=title,waitForResult=false
                        ))
                    }
                }else{
                    require(isWeb(url)){"登录网址无效"}
                    pendingWeb=LoginWeb(url,CookieStoreProviders.get()!!.getCookie(url))
                }
            }
            override fun showLoginDialog(source:BaseSource){refresh+="login"}
            override fun showSourceVariableDialog(source:BaseSource) {
                oldVerification?.showSourceVariableDialog(source)?:error("此登录动作需要源变量编辑窗口")
            }
            override fun getVerificationCode(url:String,source:BaseSource):String =
                oldVerification?.getVerificationCode(url,source)?:error("此登录动作需要验证码交互窗口")
            override fun awaitBrowser(source:BaseSource,url:String,title:String,refetchAfterSuccess:Boolean):StrResponse =
                oldVerification?.awaitBrowser(source,url,title,refetchAfterSuccess)?:SourceLogin.awaitBrowser(source,url,title,refetchAfterSuccess)
        })
        JsUiProviders.impl=object:JsUiProvider{
            override fun refresh(target:String){refresh+=target}
            override fun updateLoginData(data:Map<String,String>){updates.putAll(data)}
            override fun copy(text:String){copy=text}
        }
        Toasters.register(object:Toaster{override fun toast(message:String){messages+=message};override fun toastLong(message:String){messages+=message}})
        val result=try{
            check(source.putLoginInfo(KS_JSON.encodeToString(fields))){"无法保存登录输入"}
            if(scripts.isNotEmpty()){
                val code=source.getLoginJs().orEmpty()+"\n;"+scripts.joinToString("\n;"){"(function(){\n$it\n}).call(this);"}
                source.evalJS(code){this["result"]=fields}
            }
            LoginState(message=if(input)null else "登录操作已执行。",
                inputUpdates=if(input)emptyMap()else form.fields.filter{it.password}.associate{it.name to ""},
                web=pendingWeb)
        }catch(e:CancellationException){throw e}catch(e:Exception){LoginState(error=e.message?.take(4096)?:"登录操作失败")}
        finally{JsUiProviders.impl=oldUi;Toasters.register(oldToast);VerificationUiProviders.register(oldVerification)}
        return result.copy(messages=messages,inputUpdates=result.inputUpdates+updates,refreshTargets=refresh.toList(),copyText=copy)
    }
}

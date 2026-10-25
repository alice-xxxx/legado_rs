package io.legado.sourceengine.api

import io.legado.app.data.entities.BookSource
import io.legado.app.data.entities.rule.RowUi
import io.legado.app.help.http.CookieStoreProviders
import io.legado.app.help.source.SourceCacheProviders
import io.legado.app.help.ui.*
import io.legado.app.help.toast.*
import io.legado.app.utils.KS_JSON
import io.legado.app.utils.NetworkUtils
import io.legado.sourceengine.host.SourceEngineHostRegistry
import kotlinx.coroutines.CancellationException
import kotlinx.serialization.encodeToString

/** 登录规则只产生数据和交互请求，原生窗口由平台插件处理。 */
object SourceLogin {
    private fun isWeb(value:String)=value.startsWith("https://",true)||value.startsWith("http://",true)
    suspend fun login(r:LoginRequest):LoginState {
        val source=r.source;val url=source.getLoginJs()?.trim().orEmpty();val web=isWeb(url)
        var state=when(val action=r.action){
            LoginAction.Open->LoginState(form=form(source))
            LoginAction.Clear->{CookieStoreProviders.get()!!.removeCookieStrict(source.getKey());source.removeLoginHeader();source.removeLoginInfo();LoginState(message="已清除登录数据。")}
            is LoginAction.ImportCookies->{
                require(web&&action.url==url){"Cookie 不属于当前登录地址"}
                require(action.cookies.length<=256*1024&&action.cookies.none{it=='\n'||it=='\r'||it=='\u0000'}&&action.cookies.split(';').size<=512){"Cookie 数据无效"}
                CookieStoreProviders.get()!!.replaceCookie(url,action.cookies);LoginState(message="网页登录 Cookie 已保存。")
            }
            is LoginAction.Submit->if(web)LoginState() else submit(source,action)
        }
        val cache=SourceCacheProviders.impl!!;val key=source.getKey();val cookies=CookieStoreProviders.get()!!
        val saved=!cache.get("loginHeader_$key").isNullOrBlank()||!cache.get("userInfo_$key").isNullOrBlank()||cookies.getCookieNoSession(key).isNotBlank()
            ||!cookies.getSessionCookie(NetworkUtils.getSubDomain(key)).isNullOrBlank()||SourceEngineHostRegistry.current()!!.storage.hasCookieJarCookies(NetworkUtils.getSubDomain(key),key)
        if("login" in state.refreshTargets)state=state.copy(form=form(source))
        return state.copy(hasLoginState=saved,web=if(web&&(r.action==LoginAction.Open||r.action is LoginAction.Submit))LoginWeb(url,cookies.getCookie(url))else null)
    }
    private fun form(source:BookSource):LoginForm {
        if(isWeb(source.getLoginJs()?.trim().orEmpty()))return LoginForm("web")
        val rows=source.loginUi().orEmpty();val canLogin=!source.getLoginJs().isNullOrBlank()
        val fields=rows.mapNotNull{row->val type=when(row.type){RowUi.Type.text->"text";RowUi.Type.password->"password";RowUi.Type.select->"select";RowUi.Type.toggle->"toggle";else->return@mapNotNull null};LoginField(row.name,row.name,type,type=="password",if(type=="select")row.chars.orEmpty()else emptyList())}
        require(fields.map{it.name}.distinct().size==fields.size&&fields.size<=64){"登录字段重复或过多"}
        val actions=rows.mapIndexedNotNull{i,row->if(row.type==RowUi.Type.button&&!row.action.isNullOrBlank()&&!isWeb(row.action!!))LoginButton(i,row.name)else null}
        require(canLogin||actions.isNotEmpty()){"此书源没有可用的登录表单。"}
        val layout=rows.mapIndexedNotNull{i,row->
            val kind=when{fields.any{it.name==row.name}&&row.type!=RowUi.Type.button->"field";actions.any{it.id==i}->"action";row.type==RowUi.Type.title->"title";else->return@mapIndexedNotNull null}
            val basis=row.style?.layout_flexBasisPercent?:0F
            val cols=row.style?.cols?:when{!basis.isFinite()||basis<=0F->if(kind=="action")2 else 1;basis>=0.75F->1;basis>=0.42F->2;basis>=0.29F->3;else->4}
            LoginLayout(kind,row.name.takeIf{kind=="field"},i.takeIf{kind=="action"},row.name.takeIf{kind=="title"},cols.coerceIn(1,4),(row.style?.rows?:1).coerceIn(1,12))
        }
        return LoginForm("form",canLogin,fields,actions,layout)
    }
    private fun submit(source:BookSource,action:LoginAction.Submit):LoginState {
        val form=form(source);require(action.fields.size<=64&&action.fields.all{(name,value)->form.fields.any{it.name==name}&&value.length<=16384}){"登录字段无效"}
        val fields=form.fields.associate{it.name to action.fields[it.name].orEmpty()}
        val row=action.button?.let{source.loginUi().orEmpty().getOrNull(it)?:error("登录按钮无效")}
        if(row!=null)require(form.actions.any{it.id==action.button}){"登录按钮不可执行"}
        val messages=mutableListOf<String>();val updates=mutableMapOf<String,String>();val refresh=linkedSetOf<String>();var copy:String?=null
        val oldUi=JsUiProviders.impl;val oldToast=Toasters.current()
        JsUiProviders.impl=object:JsUiProvider{
            override fun refresh(target:String){refresh+=target}
            override fun updateLoginData(data:Map<String,String>){updates.putAll(data)}
            override fun copy(text:String){copy=text}
        }
        Toasters.register(object:Toaster{override fun toast(message:String){messages+=message};override fun toastLong(message:String){messages+=message}})
        val result=try{
            check(source.putLoginInfo(KS_JSON.encodeToString(fields))){"无法保存登录输入"}
            if(row==null)source.login()else source.evalJS(row.action!!)
            LoginState(message="登录操作已执行。",inputUpdates=form.fields.filter{it.password}.associate{it.name to ""})
        }catch(e:CancellationException){throw e}catch(e:Exception){LoginState(error=e.message?.take(4096)?:"登录操作失败")}
        finally{JsUiProviders.impl=oldUi;Toasters.register(oldToast)}
        return result.copy(messages=messages,inputUpdates=result.inputUpdates+updates,refreshTargets=refresh.toList(),copyText=copy)
    }
}

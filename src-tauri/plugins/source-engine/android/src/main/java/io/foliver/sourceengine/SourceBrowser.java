package io.foliver.sourceengine;

import android.app.*;
import android.os.*;
import android.webkit.*;
import androidx.webkit.*;
import app.tauri.plugin.*;
import io.legado.sourceengine.host.NativeCalls;
import org.json.*;
import java.util.*;

/** 独立 WebView profile 承载源页面，页面只能回传本次规则结果。 */
final class SourceBrowser {
    private final Activity activity;
    private final Handler handler=new Handler(Looper.getMainLooper());
    private final Map<String,Page> sessions=new HashMap<>();
    private final Set<Page> pages=new HashSet<>();
    SourceBrowser(Activity activity){this.activity=activity;}
    void destroy(){activity.runOnUiThread(()->{for(Page page:new ArrayList<>(pages))page.close("浏览器已销毁");});}
    private static String url(String value)throws Exception{
        android.net.Uri url=android.net.Uri.parse(value);
        if(!("http".equals(url.getScheme())||"https".equals(url.getScheme()))||url.getHost()==null||url.getUserInfo()!=null)throw new Exception("浏览器地址无效");
        return value;
    }
    void start(Invoke invoke)throws Exception{
        JSONObject input=new JSONObject(invoke.getRawArgs());String id=input.getString("sessionId");
        if(sessions.containsKey(id))throw new Exception("登录会话已存在");
        Page page=new Page(input,invoke,true);
        sessions.put(id,page);page.seed(input.optString("cookieHeader"),()->page.view.loadUrl(page.target));
        JSObject result=new JSObject();result.put("started",true);page.resolve(result);page.dialog.show();
    }
    private Page session(JSONObject input)throws Exception{
        Page page=sessions.get(input.getString("sessionId"));
        if(page!=null&&!page.input.getString("sourceId").equals(input.getString("sourceId")))throw new Exception("登录会话不属于此书源");
        return page;
    }
    void complete(Invoke invoke)throws Exception{
        Page page=session(new JSONObject(invoke.getRawArgs()));if(page==null)throw new Exception("登录会话已结束");
        JSObject result=new JSObject();
        result.put("loginUrl",page.target);result.put("sourceRevision",page.input.getLong("sourceRevision"));result.put("restoreEpoch",page.input.getLong("restoreEpoch"));result.put("cookieHeader",page.cookie());
        page.close(null);invoke.resolve(result);
    }
    void cancel(Invoke invoke)throws Exception{
        Page page=session(new JSONObject(invoke.getRawArgs()));JSObject result=new JSObject();result.put("cancelled",page!=null);
        if(page!=null)page.close(null);invoke.resolve(result);
    }
    void render(Invoke invoke)throws Exception{
        JSONObject input=new JSONObject(invoke.getRawArgs());Page page=new Page(input,invoke,false);
        StringBuilder seed=new StringBuilder();JSONArray headers=input.getJSONArray("headers");Map<String,String> requestHeaders=new HashMap<>();
        for(int i=0;i<headers.length();i++){JSONObject h=headers.getJSONObject(i);String name=h.getString("name"),value=h.getString("value");
            if(name.equalsIgnoreCase("Cookie"))seed.append(value).append(';');else requestHeaders.put(name,value);
        }
        page.seed(seed.toString(),()->{
            if(page.offline)page.view.loadDataWithBaseURL(page.target,"<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval'; style-src 'unsafe-inline'; img-src data:\">"+input.optString("html"),"text/html","UTF-8",null);
            else if(page.interactive&&!input.optString("html").isEmpty())page.view.loadDataWithBaseURL(page.target,input.optString("html"),"text/html","UTF-8",null);
            else page.view.loadUrl(page.target,requestHeaders);
        });
        if(page.interactive){page.dialog.show();if(!input.optBoolean("waitForResult",true))page.resolve(page.response("",page.target));}
    }
    private final class Page {
        final JSONObject input;final Invoke invoke;final boolean login,offline,interactive;
        final WebView view;final CookieManager cookies;final AlertDialog dialog;final String profile,target;
        boolean returned=false,closed=false,evaluated=false;
        final Runnable timeout=()->close("浏览器请求超时");
        final Runnable cancellation=new Runnable(){public void run(){
            if(closed)return;
            if(!input.optString("executionId").isEmpty()&&"true".equals(NativeCalls.cancelled(input.optString("executionId")))){close("浏览器请求已取消");return;}
            handler.postDelayed(this,100);
        }};
        Page(JSONObject input,Invoke invoke,boolean login)throws Exception{
            this.input=input;this.invoke=invoke;this.login=login;
            target=url(input.getString(login?"loginUrl":"baseUrl"));offline=!login&&"OFFLINE_SNAPSHOT".equals(input.getString("mode"));interactive=login||"INTERACTIVE_PAGE".equals(input.optString("mode"));
            if(!WebViewFeature.isFeatureSupported(WebViewFeature.MULTI_PROFILE))throw new Exception("当前系统 WebView 不支持独立书源会话，请更新 Android System WebView");
            profile="foliver-source-"+UUID.randomUUID();view=new WebView(activity);WebViewCompat.setProfile(view,profile);
            cookies=WebViewCompat.getProfile(view).getCookieManager();
            view.getSettings().setJavaScriptEnabled(true);view.getSettings().setDomStorageEnabled(!offline);view.getSettings().setAllowFileAccess(false);view.getSettings().setAllowContentAccess(false);view.getSettings().setBlockNetworkLoads(offline);view.getSettings().setCacheMode(WebSettings.LOAD_NO_CACHE);
            dialog=new AlertDialog.Builder(activity).setTitle(login?"书源登录":input.optString("title","书源页面")).setView(view).setPositiveButton(login?"返回应用":"完成",null).setNegativeButton("关闭",null).create();
            dialog.setOnShowListener(ignored->{
                dialog.getButton(AlertDialog.BUTTON_POSITIVE).setOnClickListener(v->{if(login){dialog.hide();}else evaluate();});
                dialog.getButton(AlertDialog.BUTTON_NEGATIVE).setOnClickListener(v->close("浏览器已关闭"));
            });
            dialog.setOnCancelListener(ignored->{if(login)dialog.hide();else close("浏览器已关闭");});
            String bridge="foliver_"+UUID.randomUUID().toString().replace("-","");
            if(!login){
                WebViewCompat.addWebMessageListener(view,bridge,Collections.singleton("*"),(web,message,origin,main,reply)->{
                    if(!main||closed)return;
                    try{JSONObject value=new JSONObject(message.getData());if(value.has("error")){close(value.getString("error"));return;}
                        resolve(response(value.getString("body"),offline?target:value.getString("finalUrl")));close(null);
                    }catch(Exception error){close(error.toString());}
                });
            }
            view.setTag(bridge);
            view.setWebViewClient(new WebViewClient(){
                @Override public boolean shouldOverrideUrlLoading(WebView web,WebResourceRequest request){String scheme=request.getUrl().getScheme();return offline||!("http".equals(scheme)||"https".equals(scheme));}
                @Override public void onPageFinished(WebView web,String url){if(!login&&!interactive)evaluate();}
                @Override public void onReceivedError(WebView web,WebResourceRequest request,WebResourceError error){if(request.isForMainFrame())close(error.getDescription().toString());}
                @Override public boolean onRenderProcessGone(WebView web,RenderProcessGoneDetail detail){close("浏览器渲染进程已结束");return true;}
            });
            pages.add(this);handler.postDelayed(timeout,login||!input.optBoolean("waitForResult",true)?600000:45000);if(!login&&input.optBoolean("waitForResult",true))handler.post(cancellation);
        }
        void evaluate(){
            if(evaluated||closed)return;evaluated=true;
            String script=input.optString("javaScript"),bridge=(String)view.getTag();long delay=input.optLong("delayTimeMs");
            String js="(async()=>{try{await new Promise(r=>setTimeout(r,"+delay+"));let value="+JSONObject.quote(script)+"?await(0,eval)("+JSONObject.quote(script)+"):document.documentElement.outerHTML;window["+JSONObject.quote(bridge)+"].postMessage(JSON.stringify({body:String(value??''),finalUrl:location.href}));}catch(e){window["+JSONObject.quote(bridge)+"].postMessage(JSON.stringify({error:String(e)}));}})()";
            view.evaluateJavascript(js,null);
        }
        void seed(String header,Runnable next){
            List<String> values=new ArrayList<>();for(String pair:header.split(";"))if(pair.trim().contains("="))values.add(pair.trim()+"; Path=/");
            seed(values,0,next);
        }
        void seed(List<String> values,int index,Runnable next){if(closed)return;if(index==values.size()){next.run();return;}cookies.setCookie(target,values.get(index),ignored->seed(values,index+1,next));}
        String cookie(){String value=cookies.getCookie(target);return value==null?"":value;}
        JSObject response(String body,String finalUrl){JSObject result=new JSObject();try{result.put("capabilityId",input.getString("capabilityId"));result.put("finalUrl",finalUrl);result.put("body",body);result.put("cookieHeader",offline?"":cookie());}catch(JSONException error){throw new IllegalStateException(error);}return result;}
        void resolve(JSObject result){if(!returned){returned=true;invoke.resolve(result);}}
        void close(String error){
            if(closed)return;closed=true;handler.removeCallbacks(timeout);handler.removeCallbacks(cancellation);pages.remove(this);
            if(login)sessions.remove(input.optString("sessionId"));
            if(!returned){returned=true;invoke.reject(error==null?"浏览器会话已结束":error);}
            dialog.dismiss();view.stopLoading();view.destroy();
            try{ProfileStore.getInstance().deleteProfile(profile);}catch(RuntimeException failure){android.util.Log.w("FoliverSource","清理浏览器 profile 失败",failure);}
        }
    }
}

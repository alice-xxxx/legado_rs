import Foundation
import UIKit
import WebKit
import LegadoSourceEngine
import Tauri

private struct LoginStart:Decodable{let sourceId:String;let sessionId:String;let sourceRevision:UInt64;let restoreEpoch:UInt64;let loginUrl:String;let cookieHeader:String}
private struct LoginSession:Decodable{let sourceId:String;let sessionId:String}
private struct BrowserHeader:Decodable{let name:String;let value:String}
private struct BrowserRequest:Decodable{
    let capabilityId:String;let html:String;let baseUrl:String;let javaScript:String;let delayTimeMs:UInt64;let mode:String;let method:String;let headers:[BrowserHeader];let title:String;let waitForResult:Bool;let executionId:String?;let overrideUrlRegex:String?
}
private func failure(_ text:String)->NSError{NSError(domain:"FoliverSource",code:1,userInfo:[NSLocalizedDescriptionKey:text])}
private func pageURL(_ text:String)throws->Foundation.URL{guard let url=Foundation.URL(string:text),["http","https"].contains(url.scheme ?? ""),url.host != nil,url.user == nil,url.password == nil else{throw failure("浏览器地址无效")};return url}
private func quoted(_ text:String)->String{
    guard let data=try? JSONSerialization.data(withJSONObject:[text]),let value=String(data:data,encoding:.utf8) else{return "\"\""}
    return String(value.dropFirst().dropLast())
}

/** 页面只持有自己的非持久化 WKWebsiteDataStore。 */
final class SourceBrowser {
    fileprivate var pages:[UUID:SourcePage]=[:]
    private var sessions:[String:SourcePage]=[:]
    deinit{for page in Array(pages.values){page.close("浏览器已销毁")}}
    func start(_ invoke:Invoke)throws{
        let request=try invoke.parseArgs(LoginStart.self)
        guard sessions[request.sessionId]==nil else{throw failure("登录会话已存在")}
        let page=try SourcePage(owner:self,invoke:invoke,url:pageURL(request.loginUrl),login:request,request:nil)
        sessions[request.sessionId]=page
        page.seed(request.cookieHeader){page.web.load(URLRequest(url:page.target))}
        page.show();page.resolve(["started":true])
    }
    private func session(_ request:LoginSession)throws->SourcePage?{
        let page=sessions[request.sessionId]
        if let page=page,page.login?.sourceId != request.sourceId{throw failure("登录会话不属于此书源")}
        return page
    }
    func complete(_ invoke:Invoke)throws{
        guard let page=try session(invoke.parseArgs(LoginSession.self)),let login=page.login else{throw failure("登录会话已结束")}
        page.cookies{value in
            page.close(nil)
            invoke.resolve(Tauri.JsonValue.dictionary(["loginUrl":login.loginUrl,"sourceRevision":login.sourceRevision,"restoreEpoch":login.restoreEpoch,"cookieHeader":value]))
        }
    }
    func cancel(_ invoke:Invoke)throws{
        let page=try session(invoke.parseArgs(LoginSession.self));page?.close(nil);invoke.resolve(["cancelled":page != nil])
    }
    func render(_ invoke:Invoke)throws{
        let request=try invoke.parseArgs(BrowserRequest.self)
        let page=try SourcePage(owner:self,invoke:invoke,url:pageURL(request.baseUrl),login:nil,request:request)
        let cookie=request.headers.filter{$0.name.lowercased()=="cookie"}.map{$0.value}.joined(separator:"; ")
        page.seed(cookie){
            if request.mode=="OFFLINE_SNAPSHOT"{
                let rules=#"[{"trigger":{"url-filter":".*"},"action":{"type":"block"}}]"#
                WKContentRuleListStore.default().compileContentRuleList(forIdentifier:"foliver-source-offline",encodedContentRuleList:rules){rules,error in
                    if let error=error{page.close(error.localizedDescription);return}
                    if let rules=rules{page.web.configuration.userContentController.add(rules)}
                    page.web.loadHTMLString(request.html,baseURL:page.target)
                }
            }else if !request.html.isEmpty{
                page.web.loadHTMLString(request.html,baseURL:page.target)
            }else{
                var input=URLRequest(url:page.target);for header in request.headers where header.name.lowercased() != "cookie"{input.setValue(header.value,forHTTPHeaderField:header.name)}
                page.web.load(input)
            }
        }
        if request.mode=="INTERACTIVE_PAGE"{
            page.show()
            if !request.waitForResult{page.resolve(["capabilityId":request.capabilityId,"finalUrl":request.baseUrl,"body":"","cookieHeader":""])}
        }
    }
    fileprivate func remove(_ page:SourcePage){pages.removeValue(forKey:page.id);if let login=page.login{sessions.removeValue(forKey:login.sessionId)}}
}

private final class SourcePage:UIViewController,WKNavigationDelegate,WKScriptMessageHandler,UIAdaptivePresentationControllerDelegate{
    let id=UUID();weak var owner:SourceBrowser?
    let invoke:Invoke;let target:Foundation.URL;let login:LoginStart?;let request:BrowserRequest?
    let web:WKWebView
    private let overrideUrl:NSRegularExpression?
    private let bridge="foliver_"+UUID().uuidString.replacingOccurrences(of:"-",with:"")
    private var timeout:DispatchWorkItem?;private var poll:Timer?
    private var returned=false;private var closed=false;private var evaluated=false
    init(owner:SourceBrowser,invoke:Invoke,url:Foundation.URL,login:LoginStart?,request:BrowserRequest?)throws{
        self.owner=owner;self.invoke=invoke;self.target=url;self.login=login;self.request=request
        overrideUrl=try (request?.overrideUrlRegex).map{try NSRegularExpression(pattern:"^(?:"+$0+")$")}
        let config=WKWebViewConfiguration();config.websiteDataStore = .nonPersistent();web=WKWebView(frame:CGRect(x:0,y:0,width:900,height:700),configuration:config)
        super.init(nibName:nil,bundle:nil)
        web.customUserAgent=request?.headers.first{$0.name.lowercased()=="user-agent"}?.value
        web.navigationDelegate=self;config.userContentController.add(self,name:bridge)
        self.title=login != nil ? "书源登录":request?.title
        owner.pages[id]=self
        let work=DispatchWorkItem{[weak self] in self?.close("浏览器请求超时")};timeout=work
        DispatchQueue.main.asyncAfter(deadline:.now()+(login != nil || request?.waitForResult==false ? 600:45),execute:work)
        if let execution=request?.executionId,request?.waitForResult != false{
            poll=Timer.scheduledTimer(withTimeInterval:0.1,repeats:true){[weak self] _ in if Engine.shared.isCancelled(id:execution){self?.close("浏览器请求已取消")}}
        }
    }
    required init?(coder:NSCoder){return nil}
    override func loadView(){view=web}
    func show(){
        guard let scene=UIApplication.shared.connectedScenes.compactMap({$0 as? UIWindowScene}).first(where:{$0.activationState == .foregroundActive}),var presenter=scene.windows.first(where:{$0.isKeyWindow})?.rootViewController else{close("没有可显示浏览器的活动窗口");return}
        while let next=presenter.presentedViewController{presenter=next}
        navigationItem.leftBarButtonItem=UIBarButtonItem(title:"关闭",style:.plain,target:self,action:#selector(cancel))
        navigationItem.rightBarButtonItem=UIBarButtonItem(title:login != nil ? "返回应用":"完成",style:.done,target:self,action:#selector(done))
        let navigation=UINavigationController(rootViewController:self);presenter.present(navigation,animated:true);navigation.presentationController?.delegate=self
    }
    @objc private func cancel(){close("浏览器已关闭")}
    @objc private func done(){if login != nil{navigationController?.dismiss(animated:true)}else{evaluate()}}
    func presentationControllerDidDismiss(_ presentationController:UIPresentationController){if login==nil{close("浏览器已关闭")}}
    func resolve(_ value:Tauri.JsonObject){if !returned{returned=true;invoke.resolve(Tauri.JsonValue.dictionary(value))}}
    func close(_ error:String?){
        if closed{return};closed=true;timeout?.cancel();poll?.invalidate()
        if !returned{returned=true;invoke.reject(error ?? "浏览器会话已结束")}
        web.stopLoading();web.navigationDelegate=nil;web.configuration.userContentController.removeScriptMessageHandler(forName:bridge)
        navigationController?.dismiss(animated:true);owner?.remove(self)
    }
    func seed(_ header:String,completion:@escaping()->Void){
        let items=header.split(separator:";").compactMap{part->HTTPCookie? in
            let pair=part.split(separator:"=",maxSplits:1,omittingEmptySubsequences:false);guard pair.count==2 else{return nil}
            return HTTPCookie(properties:[.domain:target.host ?? "",.path:"/",.name:pair[0].trimmingCharacters(in:.whitespaces),.value:String(pair[1]),.secure:target.scheme=="https" ? "TRUE":"FALSE"])
        }
        let group=DispatchGroup()
        for cookie in items{group.enter();web.configuration.websiteDataStore.httpCookieStore.setCookie(cookie){group.leave()}}
        group.notify(queue:.main){[weak self] in if self?.closed==false{completion()}}
    }
    func cookies(completion:@escaping(String)->Void){
        web.configuration.websiteDataStore.httpCookieStore.getAllCookies{[self] values in
            let host=(target.host ?? "").lowercased();let path=target.path.isEmpty ? "/":target.path
            let matches=values.filter{cookie in
                let domain=cookie.domain.trimmingCharacters(in:CharacterSet(charactersIn:".")).lowercased()
                return (host==domain||host.hasSuffix("."+domain))&&(!cookie.isSecure||target.scheme=="https")&&path.hasPrefix(cookie.path)&&(cookie.expiresDate==nil||cookie.expiresDate!>Date())
            }
            completion(HTTPCookie.requestHeaderFields(with:matches)["Cookie"] ?? "")
        }
    }
    func webView(_ webView:WKWebView,decidePolicyFor navigationAction:WKNavigationAction,decisionHandler:@escaping(WKNavigationActionPolicy)->Void){
        if let regex=overrideUrl,let request=request,let value=navigationAction.request.url?.absoluteString,regex.firstMatch(in:value,range:NSRange(value.startIndex...,in:value)) != nil{
            decisionHandler(.cancel)
            cookies{[self] cookie in resolve(["capabilityId":request.capabilityId,"finalUrl":value,"body":value,"cookieHeader":cookie]);close(nil)}
            return
        }
        let scheme=navigationAction.request.url?.scheme ?? ""
        decisionHandler(["http","https","about"].contains(scheme) ? .allow:.cancel)
    }
    func webView(_ webView:WKWebView,didFinish navigation:WKNavigation!){if request?.mode != "INTERACTIVE_PAGE"{evaluate()}}
    func webView(_ webView:WKWebView,didFail navigation:WKNavigation!,withError error:Error){close(error.localizedDescription)}
    func webView(_ webView:WKWebView,didFailProvisionalNavigation navigation:WKNavigation!,withError error:Error){close(error.localizedDescription)}
    func webViewWebContentProcessDidTerminate(_ webView:WKWebView){close("浏览器渲染进程已结束")}
    private func evaluate(){
        guard !closed,!evaluated,let request=request else{return};evaluated=true
        let script=quoted(request.javaScript),name=quoted(bridge)
        let capture=overrideUrl==nil ? "window.webkit.messageHandlers[\(name)].postMessage({body:String(value??''),finalUrl:location.href});":""
        let source="(async()=>{try{await new Promise(r=>setTimeout(r,\(request.delayTimeMs)));let value=\(script)?await(0,eval)(\(script)):document.documentElement.outerHTML;\(capture)}catch(e){window.webkit.messageHandlers[\(name)].postMessage({error:String(e)});}})()"
        web.evaluateJavaScript(source+";void 0;"){[weak self] _,error in if let error=error{self?.close(error.localizedDescription)}}
    }
    func userContentController(_ userContentController:WKUserContentController,didReceive message:WKScriptMessage){
        guard !closed,message.frameInfo.isMainFrame,let value=message.body as? [String:Any],let request=request else{return}
        if let error=value["error"] as? String{close(error);return}
        guard let body=value["body"] as? String,body.utf8.count<=16*1024*1024,let final=value["finalUrl"] as? String else{close("浏览器结果无效");return}
        cookies{[self] cookie in
            resolve(["capabilityId":request.capabilityId,"finalUrl":request.mode=="OFFLINE_SNAPSHOT" ? request.baseUrl:final,"body":body,"cookieHeader":request.mode=="OFFLINE_SNAPSHOT" ? "":cookie])
            close(nil)
        }
    }
}

import Foundation
import LegadoSourceEngine
import SwiftRs
import Tauri

private struct CallContext:Decodable {let dataDirectory:String;let executionId:String}

/** Tauri 只转换参数；具名 KMP API 直接返回对应结果。 */
final class SourceEnginePlugin:Plugin {
    private let browser=SourceBrowser()
    private func call<Q,T>(_ invoke:Invoke,decode:(String)throws->Q,run:(Q,String,String,@escaping(T?,Error?)->Void)->Void,encode:@escaping(T)throws->String)throws{
        let context=try invoke.parseArgs(CallContext.self)
        let request=try decode(invoke.getRawArgs())
        run(request,context.dataDirectory,context.executionId){result,error in
            if let error=error{invoke.reject(error.localizedDescription);return}
            do{guard let result=result else{throw NSError(domain:"FoliverSource",code:1,userInfo:[NSLocalizedDescriptionKey:"引擎未返回结果"])}
                guard let value=try JSONSerialization.jsonObject(with:Data(encode(result).utf8)) as? [String:Any] else{throw NSError(domain:"FoliverSource",code:1,userInfo:[NSLocalizedDescriptionKey:"引擎结果不是对象"])}
                invoke.resolve(JSTypes.coerceDictionaryToJSObject(value))
            }catch{invoke.reject(error.localizedDescription)}
        }
    }
    @objc public func search(_ invoke:Invoke)throws{try call(invoke,decode:NativeCodec.shared.decodeSearchRequest,run:Engine.shared.search,encode:NativeCodec.shared.encodeSearchPage)}
    @objc public func exploreKinds(_ invoke:Invoke)throws{try call(invoke,decode:NativeCodec.shared.decodeSourceRequest,run:Engine.shared.exploreKinds,encode:NativeCodec.shared.encodeExploreKinds)}
    @objc public func explore(_ invoke:Invoke)throws{try call(invoke,decode:NativeCodec.shared.decodeExploreRequest,run:Engine.shared.explore,encode:NativeCodec.shared.encodeSearchPage)}
    @objc public func detail(_ invoke:Invoke)throws{try call(invoke,decode:NativeCodec.shared.decodeBookRequest,run:Engine.shared.detail,encode:NativeCodec.shared.encodeBook)}
    @objc public func chapters(_ invoke:Invoke)throws{try call(invoke,decode:NativeCodec.shared.decodeBookRequest,run:Engine.shared.chapters,encode:NativeCodec.shared.encodeChapterList)}
    @objc public func content(_ invoke:Invoke)throws{try call(invoke,decode:NativeCodec.shared.decodeContentRequest,run:Engine.shared.content,encode:NativeCodec.shared.encodeChapterContent)}
    @objc public func resolveMedia(_ invoke:Invoke)throws{try call(invoke,decode:NativeCodec.shared.decodeMediaRequest,run:Engine.shared.resolveMedia,encode:NativeCodec.shared.encodeMediaLocation)}
    @objc public func httpTtsAudio(_ invoke:Invoke)throws{try call(invoke,decode:NativeCodec.shared.decodeTtsRequest,run:Engine.shared.httpTtsAudio,encode:NativeCodec.shared.encodeAudioData)}
    @objc public func login(_ invoke:Invoke)throws{try call(invoke,decode:NativeCodec.shared.decodeLoginRequest,run:Engine.shared.login,encode:NativeCodec.shared.encodeLoginState)}
    @objc public func renderBrowser(_ invoke:Invoke){DispatchQueue.main.async{do{try self.browser.render(invoke)}catch{invoke.reject(error.localizedDescription)}}}
    @objc public func startWebLogin(_ invoke:Invoke){DispatchQueue.main.async{do{try self.browser.start(invoke)}catch{invoke.reject(error.localizedDescription)}}}
    @objc public func completeWebLogin(_ invoke:Invoke){DispatchQueue.main.async{do{try self.browser.complete(invoke)}catch{invoke.reject(error.localizedDescription)}}}
    @objc public func cancelWebLogin(_ invoke:Invoke){DispatchQueue.main.async{do{try self.browser.cancel(invoke)}catch{invoke.reject(error.localizedDescription)}}}
}

@_cdecl("init_plugin_source_engine")
public func initPlugin()->Plugin{SourceEnginePlugin()}

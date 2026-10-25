package io.foliver.sourceengine;

import android.app.Activity;
import androidx.appcompat.app.AppCompatActivity;
import app.tauri.annotation.Command;
import app.tauri.annotation.TauriPlugin;
import app.tauri.plugin.*;
import io.legado.sourceengine.api.*;
import org.json.JSONObject;
import java.util.concurrent.*;

/** Tauri 参数转换；规则能力由 KMP 的具名 API 实现。 */
@TauriPlugin
public final class SourceEnginePlugin extends Plugin {
    private final Activity activity;
    private final ExecutorService workers=Executors.newCachedThreadPool();
    private final SourceBrowser browser;
    public SourceEnginePlugin(Activity activity){super(activity);this.activity=activity;browser=new SourceBrowser(activity);}
    @Override public void onDestroy(AppCompatActivity activity){browser.destroy();workers.shutdownNow();}
    private interface Decode<Q>{Q run(String json)throws Exception;}
    private interface Run<Q,T>{T run(Q request,String directory,String id)throws Exception;}
    private interface Encode<T>{String run(T result)throws Exception;}
    private <Q,T> void call(Invoke invoke,Decode<Q> decode,Run<Q,T> run,Encode<T> encode){
        workers.execute(()->{try{
            JSONObject input=new JSONObject(invoke.getRawArgs());
            T result=run.run(decode.run(input.toString()),input.getString("dataDirectory"),input.getString("executionId"));
            invoke.resolve(new JSObject(encode.run(result)));
        }catch(Exception error){invoke.reject(error.toString());}});
    }
    @Command public void search(Invoke invoke){call(invoke,NativeCodec::decodeSearchRequest,Engine::search,NativeCodec::encodeSearchPage);}
    @Command public void exploreKinds(Invoke invoke){call(invoke,NativeCodec::decodeSourceRequest,Engine::exploreKinds,NativeCodec::encodeExploreKinds);}
    @Command public void explore(Invoke invoke){call(invoke,NativeCodec::decodeExploreRequest,Engine::explore,NativeCodec::encodeSearchPage);}
    @Command public void detail(Invoke invoke){call(invoke,NativeCodec::decodeBookRequest,Engine::detail,NativeCodec::encodeBook);}
    @Command public void chapters(Invoke invoke){call(invoke,NativeCodec::decodeBookRequest,Engine::chapters,NativeCodec::encodeChapterList);}
    @Command public void content(Invoke invoke){call(invoke,NativeCodec::decodeContentRequest,Engine::content,NativeCodec::encodeChapterContent);}
    @Command public void resolveMedia(Invoke invoke){call(invoke,NativeCodec::decodeMediaRequest,Engine::resolveMedia,NativeCodec::encodeMediaLocation);}
    @Command public void httpTtsAudio(Invoke invoke){call(invoke,NativeCodec::decodeTtsRequest,Engine::httpTtsAudio,NativeCodec::encodeAudioData);}
    @Command public void login(Invoke invoke){call(invoke,NativeCodec::decodeLoginRequest,Engine::login,NativeCodec::encodeLoginState);}
    @Command public void renderBrowser(Invoke invoke){activity.runOnUiThread(()->{try{browser.render(invoke);}catch(Exception error){invoke.reject(error.toString());}});}
    @Command public void startWebLogin(Invoke invoke){activity.runOnUiThread(()->{try{browser.start(invoke);}catch(Exception error){invoke.reject(error.toString());}});}
    @Command public void completeWebLogin(Invoke invoke){activity.runOnUiThread(()->{try{browser.complete(invoke);}catch(Exception error){invoke.reject(error.toString());}});}
    @Command public void cancelWebLogin(Invoke invoke){activity.runOnUiThread(()->{try{browser.cancel(invoke);}catch(Exception error){invoke.reject(error.toString());}});}
}

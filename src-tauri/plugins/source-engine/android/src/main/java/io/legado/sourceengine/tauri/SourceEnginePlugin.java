package io.legado.sourceengine.tauri;

import android.app.Activity;
import app.tauri.annotation.Command;
import app.tauri.annotation.TauriPlugin;
import app.tauri.plugin.Invoke;
import app.tauri.plugin.JSObject;
import app.tauri.plugin.Plugin;
import io.legado.sourceengine.android.AndroidSourceWebLogin;
import io.legado.sourceengine.bridge.AndroidSourceEngineBlocking;
import io.legado.sourceengine.bridge.NativeDtos;
import org.json.JSONObject;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/** Invoke KMP on a worker; its HTTP and persistent storage callbacks are implemented by Rust. */
@TauriPlugin
public final class SourceEnginePlugin extends Plugin {
    private final Activity activity;
    private final ExecutorService executor = Executors.newCachedThreadPool();
    private final ExecutorService loginExecutor = Executors.newSingleThreadExecutor();

    public SourceEnginePlugin(Activity activity) {
        super(activity);
        this.activity = activity;
    }

    @Command
    public void search(Invoke invoke) {
        call(invoke, NativeDtos::decodeSearchRequest, AndroidSourceEngineBlocking::search, NativeDtos::encodeSearchPage);
    }

    @Command
    public void exploreKinds(Invoke invoke) {
        call(invoke, NativeDtos::decodeSourceRequest, AndroidSourceEngineBlocking::exploreKinds, NativeDtos::encodeExploreKinds);
    }

    @Command
    public void explore(Invoke invoke) {
        call(invoke, NativeDtos::decodeExploreRequest, AndroidSourceEngineBlocking::explore, NativeDtos::encodeSearchPage);
    }

    @Command
    public void detail(Invoke invoke) {
        call(invoke, NativeDtos::decodeBookRequest, AndroidSourceEngineBlocking::detail, NativeDtos::encodeBook);
    }

    @Command
    public void chapters(Invoke invoke) {
        call(invoke, NativeDtos::decodeBookRequest, AndroidSourceEngineBlocking::chapters, NativeDtos::encodeChapterList);
    }

    @Command
    public void content(Invoke invoke) {
        call(invoke, NativeDtos::decodeContentRequest, AndroidSourceEngineBlocking::content, NativeDtos::encodeChapterContent);
    }

    @Command
    public void resolveMedia(Invoke invoke) {
        call(invoke, NativeDtos::decodeMediaRequest, AndroidSourceEngineBlocking::resolveMedia, NativeDtos::encodeMediaLocation);
    }

    @Command
    public void httpTtsAudio(Invoke invoke) {
        call(invoke, NativeDtos::decodeTtsRequest, AndroidSourceEngineBlocking::httpTtsAudio, NativeDtos::encodeAudioData);
    }

    @Command
    public void login(Invoke invoke) {
        call(invoke, NativeDtos::decodeLoginRequest, AndroidSourceEngineBlocking::login, NativeDtos::encodeLoginState);
    }

    @FunctionalInterface
    private interface Codec<I, O> { O run(I input) throws Exception; }

    @FunctionalInterface
    private interface NativeCall<Q, R> {
        R run(Q request, String directory, String executionId, android.content.Context context) throws Exception;
    }

    private <Q, R> void call(Invoke invoke, Codec<String, Q> decode, NativeCall<Q, R> run, Codec<R, String> encode) {
        executor.execute(() -> {
            try {
                EngineContextArgs context = invoke.parseArgs(EngineContextArgs.class);
                R result = run.run(decode.run(invoke.getRawArgs()), context.dataDirectory, context.executionId, activity);
                invoke.resolve(new JSObject(encode.run(result)));
            } catch (Throwable error) {
                reject(invoke, error);
            }
        });
    }

    @Command
    public void startWebLogin(Invoke invoke) {
        final SourceLoginWebStartArgs args = invoke.parseArgs(SourceLoginWebStartArgs.class);
        loginExecutor.execute(() -> {
            try {
                boolean started = AndroidSourceWebLogin.start(
                    activity,
                    args.sourceId,
                    args.sessionId,
                    args.sourceRevision,
                    args.restoreEpoch,
                    args.loginUrl,
                    args.cookieHeader
                );
                if (!started) {
                    invoke.reject("Native source login browser did not start");
                    return;
                }
                JSObject response = new JSObject();
                response.put("started", true);
                invoke.resolve(response);
            } catch (Throwable error) {
                reject(invoke, error);
            }
        });
    }

    @Command
    public void completeWebLogin(Invoke invoke) {
        final SourceLoginWebSessionArgs args = invoke.parseArgs(SourceLoginWebSessionArgs.class);
        loginExecutor.execute(() -> {
            try {
                JSONObject capture = new JSONObject(
                    AndroidSourceWebLogin.complete(activity, args.sourceId, args.sessionId)
                );
                JSObject response = new JSObject();
                response.put("loginUrl", capture.getString("loginUrl"));
                response.put("sourceRevision", capture.getLong("sourceRevision"));
                response.put("restoreEpoch", capture.getLong("restoreEpoch"));
                response.put("cookieHeader", capture.getString("cookieHeader"));
                invoke.resolve(response);
            } catch (Throwable error) {
                reject(invoke, error);
            }
        });
    }

    @Command
    public void cancelWebLogin(Invoke invoke) {
        final SourceLoginWebSessionArgs args = invoke.parseArgs(SourceLoginWebSessionArgs.class);
        loginExecutor.execute(() -> {
            try {
                JSObject response = new JSObject();
                response.put(
                    "cancelled",
                    AndroidSourceWebLogin.cancel(activity, args.sourceId, args.sessionId)
                );
                invoke.resolve(response);
            } catch (Throwable error) {
                reject(invoke, error);
            }
        });
    }

    private static void reject(Invoke invoke, Throwable error) {
        String message = error.getMessage();
        invoke.reject(message == null ? error.toString() : message);
    }
}

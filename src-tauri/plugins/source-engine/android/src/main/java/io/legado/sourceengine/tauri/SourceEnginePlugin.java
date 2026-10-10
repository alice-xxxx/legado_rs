package io.legado.sourceengine.tauri;

import android.app.Activity;
import app.tauri.annotation.Command;
import app.tauri.annotation.TauriPlugin;
import app.tauri.plugin.Invoke;
import app.tauri.plugin.JSObject;
import app.tauri.plugin.Plugin;
import io.legado.sourceengine.android.AndroidSourceWebLogin;
import io.legado.sourceengine.bridge.AndroidSourceEngineBlocking;
import org.json.JSONObject;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/** Invoke KMP on a worker; its HTTP and persistent storage callbacks are implemented by Rust. */
@TauriPlugin
public final class SourceEnginePlugin extends Plugin {
    private final Activity activity;
    private final ExecutorService executor = Executors.newSingleThreadExecutor();
    private final ExecutorService loginExecutor = Executors.newSingleThreadExecutor();

    public SourceEnginePlugin(Activity activity) {
        super(activity);
        this.activity = activity;
    }

    @Command
    public void execute(Invoke invoke) {
        final ExecuteArgs args = invoke.parseArgs(ExecuteArgs.class);
        final String appDataDirectory = new java.io.File(activity.getFilesDir(), "source-engine").getAbsolutePath();

        executor.execute(() -> {
            try {
                String result = AndroidSourceEngineBlocking.executeJson(args.requestJson, appDataDirectory, activity);
                JSObject response = new JSObject();
                response.put("resultJson", result);
                invoke.resolve(response);
            } catch (Throwable error) {
                String message = error.getMessage();
                invoke.reject(message == null ? error.toString() : message);
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

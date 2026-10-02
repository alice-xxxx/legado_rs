package io.legado.sourceengine.tauri;

import android.app.Activity;
import app.tauri.annotation.Command;
import app.tauri.annotation.TauriPlugin;
import app.tauri.plugin.Invoke;
import app.tauri.plugin.JSObject;
import app.tauri.plugin.Plugin;
import io.legado.sourceengine.bridge.AndroidSourceEngineBlocking;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/** Invoke KMP on a worker; its HTTP and persistent storage callbacks are implemented by Rust. */
@TauriPlugin
public final class SourceEnginePlugin extends Plugin {
    private final Activity activity;
    private final ExecutorService executor = Executors.newSingleThreadExecutor();

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
                String result = AndroidSourceEngineBlocking.executeJson(args.requestJson, appDataDirectory);
                JSObject response = new JSObject();
                response.put("resultJson", result);
                invoke.resolve(response);
            } catch (Throwable error) {
                String message = error.getMessage();
                invoke.reject(message == null ? error.toString() : message);
            }
        });
    }
}

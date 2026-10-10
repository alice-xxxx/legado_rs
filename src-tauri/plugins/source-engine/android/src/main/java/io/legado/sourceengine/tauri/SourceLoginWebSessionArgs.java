package io.legado.sourceengine.tauri;

import app.tauri.annotation.InvokeArg;

@InvokeArg
public final class SourceLoginWebSessionArgs {
    public String sourceId;
    public String sessionId;
}

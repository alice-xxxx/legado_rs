package io.legado.sourceengine.tauri;

import app.tauri.annotation.InvokeArg;

@InvokeArg
public final class SourceLoginWebStartArgs {
    public String sourceId;
    public String sessionId;
    public long sourceRevision;
    public long restoreEpoch;
    public String loginUrl;
    public String cookieHeader;
}

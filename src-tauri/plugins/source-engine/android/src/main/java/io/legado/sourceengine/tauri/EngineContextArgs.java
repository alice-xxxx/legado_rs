package io.legado.sourceengine.tauri;

import app.tauri.annotation.InvokeArg;
import com.fasterxml.jackson.annotation.JsonIgnoreProperties;

@InvokeArg
@JsonIgnoreProperties(ignoreUnknown = true)
public final class EngineContextArgs {
    public String dataDirectory;
    public String executionId;
}

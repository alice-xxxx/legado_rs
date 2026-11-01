package io.legado.app.utils

internal actual fun parseScriptUrl(url: String, baseUrl: String?): ScriptUrlParts =
    ScriptUrlCompat.parse(url, baseUrl)

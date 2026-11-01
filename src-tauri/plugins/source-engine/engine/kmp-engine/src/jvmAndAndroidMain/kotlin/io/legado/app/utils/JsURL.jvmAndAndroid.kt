package io.legado.app.utils

import java.net.URL

@Suppress("DEPRECATION")
internal actual fun parseScriptUrl(url: String, baseUrl: String?): ScriptUrlParts {
    val parsed = if (baseUrl.isNullOrEmpty()) URL(url) else URL(URL(baseUrl), url)
    val origin = parsed.protocol + "://" + parsed.host + if (parsed.port > 0) ":" + parsed.port else ""
    return ScriptUrlParts(parsed.host, origin, parsed.path, parsed.query)
}

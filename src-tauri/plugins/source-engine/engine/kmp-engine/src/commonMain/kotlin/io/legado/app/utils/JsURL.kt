package io.legado.app.utils

class JsURL(url: String, baseUrl: String? = null) {
    private val parts = parseScriptUrl(url, baseUrl)
    val host: String = parts.host
    val origin: String = parts.origin
    val pathname: String = parts.pathname
    val searchParams: Map<String, String>? = scriptUrlQuery(parts.query)
}

internal data class ScriptUrlParts(
    val host: String,
    val origin: String,
    val pathname: String,
    val query: String?,
)

internal expect fun parseScriptUrl(url: String, baseUrl: String?): ScriptUrlParts

internal fun scriptUrlQuery(query: String?): Map<String, String>? = query?.let {
    buildMap {
        for (segment in query.split('&')) {
            val index = segment.indexOf('=')
            if (index >= 0) put(segment.substring(0, index), formUrlDecode(segment.substring(index + 1)))
        }
    }
}

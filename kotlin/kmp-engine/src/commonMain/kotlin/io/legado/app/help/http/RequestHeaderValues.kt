package io.legado.app.help.http

import io.legado.app.utils.PercentCodec

// Keep URI delimiters, existing %XX escapes and control characters unchanged.
// Leave control characters for the HTTP client's normal validation.
private val refererCodec = PercentCodec.of(
    (0..127).filter { it != ' '.code }.joinToString("") { it.toChar().toString() }
)

internal fun requestHeaderValue(name: String, value: String): String {
    if (!name.equals("Referer", ignoreCase = true) ||
        value.none { it == ' ' || it.code > 127 }
    ) return value
    return refererCodec.encode(value) { it.encodeToByteArray() }
}

/** Add source headers after setting the request URL; preserve opaque Cookie values. */
internal fun KmpRequestBuilder.addRequestHeader(name: String, value: String): KmpRequestBuilder {
    val normalized = requestHeaderValue(name, value)
    if (name.equals("Cookie", ignoreCase = true)) {
        require(normalized.none { (it.code < 32 && it != '\t') || it.code == 127 }) {
            "Invalid control character in Cookie header"
        }
        if (normalized.any { it.code > 127 }) {
            return addNonAsciiRequestHeader(name, normalized)
        }
    }
    return addHeader(name, normalized)
}

// OkHttp's Request.Builder validates values before interceptors can run.
// Platform implementations retain existing headers and add the validated value verbatim.
internal expect fun KmpRequestBuilder.addNonAsciiRequestHeader(
    name: String,
    value: String,
): KmpRequestBuilder

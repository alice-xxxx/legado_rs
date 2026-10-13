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

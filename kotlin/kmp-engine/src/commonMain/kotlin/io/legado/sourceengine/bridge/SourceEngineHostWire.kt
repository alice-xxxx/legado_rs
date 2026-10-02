package io.legado.sourceengine.bridge

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import okio.ByteString.Companion.decodeBase64
import okio.ByteString.Companion.toByteString

/**
 * Mobile JNI and cinterop carry the same JSON wire format. Keeping the codec in commonMain
 * prevents platform adapters from diverging on byte encoding, redirect options, or null values.
 */
object SourceEngineHostWire {
    fun encodeHttpRequest(request: HostHttpRequest): String = buildJsonObject {
        put("url", request.url)
        put("method", request.method)
        put("headers", buildJsonArray {
            request.headers.forEach { header ->
                add(buildJsonObject {
                    put("name", header.name)
                    put("value", header.value)
                })
            }
        })
        request.body?.let { put("bodyBase64", it.toByteString().base64()) }
        if (request.multipart.isNotEmpty()) {
            put("multipart", buildJsonArray {
                request.multipart.forEach { part ->
                    add(buildJsonObject {
                        put("name", part.name)
                        part.text?.let { put("text", it) }
                        part.fileName?.let { put("fileName", it) }
                        part.contentType?.let { put("contentType", it) }
                        part.bytes?.let { put("bytesBase64", it.toByteString().base64()) }
                    })
                }
            })
        }
        request.connectTimeoutMs?.let { put("connectTimeoutMs", it) }
        request.readTimeoutMs?.let { put("readTimeoutMs", it) }
        request.callTimeoutMs?.let { put("callTimeoutMs", it) }
        request.maxRedirects?.let { put("maxRedirects", it) }
        put("useCookieJar", request.useCookieJar)
        request.proxy?.let { put("proxy", it) }
        put("acceptInvalidCerts", request.acceptInvalidCerts)
    }.toString()

    fun decodeHttpResponse(json: String, fallbackUrl: String): HostHttpResponse {
        val payload = decodeEnvelope(json).jsonObject
        val headers = payload["headers"]?.jsonArray.orEmpty().map { item ->
            HostHeader(
                name = item.jsonObject["name"].requiredString("header name"),
                value = item.jsonObject["value"].requiredString("header value"),
            )
        }
        val body = payload["bodyBase64"]?.jsonPrimitive?.contentOrNull.orEmpty()
            .decodeBase64()?.toByteArray()
            ?: error("Rust HTTP response contains invalid base64 body")
        return HostHttpResponse(
            requestedUrl = payload["requestedUrl"]?.jsonPrimitive?.contentOrNull ?: fallbackUrl,
            finalUrl = payload["finalUrl"].requiredString("finalUrl"),
            status = payload["status"]?.jsonPrimitive?.contentOrNull?.toIntOrNull()
                ?: error("Rust HTTP response has no valid status"),
            reason = payload["reason"]?.jsonPrimitive?.contentOrNull.orEmpty(),
            headers = headers,
            body = body,
        )
    }

    fun encodeStorageRequest(
        namespace: String,
        operation: String,
        key: String? = null,
        value: String? = null,
        ttlSeconds: Long? = null,
        keyPrefixes: List<String>? = null,
        contains: String? = null,
    ): String = buildJsonObject {
        put("namespace", namespace)
        put("operation", operation)
        key?.let { put("key", it) }
        value?.let { put("value", it) }
        ttlSeconds?.let { put("ttlSeconds", it) }
        keyPrefixes?.let { prefixes -> put("keyPrefixes", JsonArray(prefixes.map(::JsonPrimitive))) }
        contains?.let { put("contains", it) }
    }.toString()

    fun decodeStorageResponse(json: String): String? {
        val result = decodeEnvelope(json)
        val storage = result.jsonObject
        return when (val value = storage["value"]) {
            null, JsonNull -> null
            else -> value.jsonPrimitive.contentOrNull
        }
    }

    private fun decodeEnvelope(json: String): JsonObject {
        val envelope = kotlinx.serialization.json.Json.parseToJsonElement(json).jsonObject
        if (envelope["ok"]?.jsonPrimitive?.contentOrNull != "true") {
            error(envelope["error"]?.jsonPrimitive?.contentOrNull ?: "Rust host request failed")
        }
        return envelope["value"]?.jsonObject ?: error("Rust host response has no value")
    }

    private fun kotlinx.serialization.json.JsonElement?.requiredString(label: String): String =
        this?.jsonPrimitive?.contentOrNull ?: error("Rust response has no $label")
}

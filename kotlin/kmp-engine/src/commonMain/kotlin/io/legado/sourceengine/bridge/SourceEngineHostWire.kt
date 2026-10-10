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
    const val MAX_BROWSER_HTML_BYTES: Int = 2 * 1024 * 1024
    const val MAX_BROWSER_SCRIPT_BYTES: Int = 512 * 1024
    const val MAX_BROWSER_RESULT_BYTES: Int = 4 * 1024 * 1024
    const val MAX_BROWSER_DELAY_MS: Long = 15_000

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
        request.cookieScope?.let { put("cookieScope", it) }
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
        val redirects = payload["redirects"]?.jsonArray.orEmpty().map { item ->
            val redirect = item.jsonObject
            HostHttpRedirect(
                fromUrl = redirect["fromUrl"].requiredString("redirect fromUrl"),
                toUrl = redirect["toUrl"].requiredString("redirect toUrl"),
                status = redirect["status"]?.jsonPrimitive?.contentOrNull?.toIntOrNull()
                    ?: error("Rust HTTP redirect has no valid status"),
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
            redirects = redirects,
            headers = headers,
            body = body,
        )
    }

    fun encodeBrowserRequest(request: HostBrowserRequest): String {
        validateBrowserRequest(request)
        return buildJsonObject {
            put("capabilityId", request.capabilityId)
            put("mode", request.mode.name.lowercase())
            put("baseUrl", request.baseUrl)
            put("delayTimeMs", request.delayTimeMs)
            put("method", request.method)
            put("headers", buildJsonArray {
                request.headers.forEach { header ->
                    add(buildJsonObject {
                        put("name", header.name)
                        put("value", header.value)
                    })
                }
            })
            put("useCookieJar", request.useCookieJar)
            request.cookieScope?.let { put("cookieScope", it) }
            put("prepareScript", SourceBrowserSnapshot.prepareDocument(request))
            put("evaluateScript", SourceBrowserSnapshot.evaluateRule(request))
        }.toString()
    }

    fun decodeBrowserResponse(json: String, request: HostBrowserRequest): HostBrowserResponse {
        val value = decodeBrowserCallbackEnvelope(json).jsonObject
        val capabilityId = value["capabilityId"].requiredString("browser capabilityId")
        check(capabilityId == request.capabilityId) {
            "Private browser result capability did not match the request"
        }
        val finalUrl = value["finalUrl"].requiredString("browser finalUrl")
        if (request.mode == HostBrowserMode.OFFLINE_SNAPSHOT) {
            check(finalUrl == request.baseUrl) {
                "Private browser snapshot changed its final URL"
            }
        } else {
            check(finalUrl.startsWith("https://", ignoreCase = true) ||
                finalUrl.startsWith("http://", ignoreCase = true)) {
                "Private dynamic browser returned a non-HTTP(S) final URL"
            }
        }
        val body = value["body"].requiredString("browser body")
        check(body.encodeToByteArray().size <= MAX_BROWSER_RESULT_BYTES) {
            "Private browser result exceeds the 4 MiB limit"
        }
        return HostBrowserResponse(capabilityId, finalUrl, body)
    }

    fun verifyBrowserDocumentPrepared(json: String, request: HostBrowserRequest) {
        val value = decodeBrowserCallbackEnvelope(json).jsonObject
        val capabilityId = value["capabilityId"].requiredString("browser preparation capabilityId")
        check(capabilityId == request.capabilityId) {
            "Private browser document preparation did not match the request"
        }
    }

    fun validateBrowserRequest(request: HostBrowserRequest) {
        check(request.capabilityId.matches(Regex("[0-9a-f]{32}"))) {
            "Private browser capability ID is invalid"
        }
        check(request.baseUrl.startsWith("https://", ignoreCase = true) ||
            request.baseUrl.startsWith("http://", ignoreCase = true)) {
            "Private browser base URL must be HTTP(S)"
        }
        check(request.html.encodeToByteArray().size <= MAX_BROWSER_HTML_BYTES) {
            "Private browser HTML exceeds the 2 MiB limit"
        }
        check(request.javaScript.encodeToByteArray().size <= MAX_BROWSER_SCRIPT_BYTES) {
            "Private browser rule exceeds the 512 KiB limit"
        }
        check(request.delayTimeMs in 0..MAX_BROWSER_DELAY_MS) {
            "Private browser delay must be between 0 and 15000 ms"
        }
        if (request.mode == HostBrowserMode.DYNAMIC_PAGE) {
            check(request.method.equals("GET", ignoreCase = true)) {
                "Dynamic source browser currently supports GET only; POST pages use the offline response snapshot"
            }
            check(request.html.isEmpty()) {
                "Dynamic source browser does not accept an injected HTML document"
            }
            check(request.headers.sumOf { it.name.encodeToByteArray().size + it.value.encodeToByteArray().size } <= 64 * 1024) {
                "Private dynamic browser headers exceed the 64 KiB limit"
            }
        }
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

    private fun decodeBrowserCallbackEnvelope(json: String): JsonObject {
        val parsed = kotlinx.serialization.json.Json.parseToJsonElement(json)
        val normalized = if (parsed is JsonPrimitive && parsed.isString) parsed.content else json
        return decodeEnvelope(normalized)
    }

    private fun kotlinx.serialization.json.JsonElement?.requiredString(label: String): String =
        this?.jsonPrimitive?.contentOrNull ?: error("Rust response has no $label")
}

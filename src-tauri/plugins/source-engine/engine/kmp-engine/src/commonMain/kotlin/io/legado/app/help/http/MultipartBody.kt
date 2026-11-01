package io.legado.app.help.http

import io.legado.app.utils.InputStream
import io.legado.app.utils.asSource
import io.legado.app.utils.randomUUIDString
import okio.BufferedSink
import okio.use

internal data class MultipartPart(val name: String, val filename: String?, val body: KmpRequestBody)

fun KmpRequestBuilder.postMultipart(type: String?, form: Map<String, Any>) {
    val parts = form.map { (name, value) ->
        if (value is Map<*, *>) {
            MultipartPart(name, value["fileName"] as String,
                multipartValueBody(value["file"], (value["contentType"] as? String)?.toKmpMediaType()))
        } else MultipartPart(name, null, value.toString().toKmpRequestBody(null))
    }
    post(createMultipartBody(parts, type ?: "multipart/mixed", randomUUIDString()))
}

internal expect fun multipartValueBody(value: Any?, contentType: KmpMediaType?): KmpRequestBody
internal expect fun createMultipartBody(parts: List<MultipartPart>, type: String, boundary: String): KmpRequestBody

internal fun streamMultipartBody(input: InputStream, type: String): KmpRequestBody = object : KmpRequestBody() {
    private val mediaType = type.toKmpMediaType()
    override fun contentType(): KmpMediaType = mediaType
    override fun isOneShot(): Boolean = true
    override fun writeTo(sink: BufferedSink) {
        input.asSource().use { sink.writeAll(it) }
    }
}

/** Native 没有 OkHttp multipart 编码器；此最小后备编码共用于两种书源请求入口。 */
internal fun writeMultipart(parts: List<MultipartPart>, boundary: String, sink: BufferedSink) {
    check(parts.isNotEmpty()) { "Multipart body must have at least one part." }
    fun quote(value: String) = "\"" + value.replace("\n", "%0A").replace("\r", "%0D").replace("\"", "%22") + "\""
    for (part in parts) {
        sink.writeUtf8("--").writeUtf8(boundary).writeUtf8("\r\nContent-Disposition: form-data; name=")
            .writeUtf8(quote(part.name))
        part.filename?.let { sink.writeUtf8("; filename=").writeUtf8(quote(it)) }
        sink.writeUtf8("\r\n")
        part.body.contentType()?.let { sink.writeUtf8("Content-Type: ").writeUtf8(it.toString()).writeUtf8("\r\n") }
        sink.writeUtf8("\r\n")
        part.body.writeTo(sink)
        sink.writeUtf8("\r\n")
    }
    sink.writeUtf8("--").writeUtf8(boundary).writeUtf8("--\r\n")
}

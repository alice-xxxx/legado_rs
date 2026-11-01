package io.legado.app.help.http

import okio.Buffer

internal actual fun multipartValueBody(value: Any?, contentType: KmpMediaType?): KmpRequestBody =
    if (value is ByteArray) value.toKmpRequestBody(contentType) else value.toString().toKmpRequestBody(contentType)

internal actual fun createMultipartBody(parts: List<MultipartPart>, type: String, boundary: String): KmpRequestBody {
    require(type.substringBefore('/').equals("multipart", true)) { "Expected multipart media type" }
    val bytes = Buffer().apply { writeMultipart(parts, boundary, this) }.readByteArray()
    return bytes.toKmpRequestBody((type + "; boundary=" + boundary).toKmpMediaType())
}

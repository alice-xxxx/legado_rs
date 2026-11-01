package io.legado.app.help.http

import okhttp3.MultipartBody
import okhttp3.RequestBody.Companion.asRequestBody
import java.io.File

internal actual fun multipartValueBody(value: Any?, contentType: KmpMediaType?): KmpRequestBody = when (value) {
    is File -> value.asRequestBody(contentType)
    is ByteArray -> value.toKmpRequestBody(contentType)
    else -> value.toString().toKmpRequestBody(contentType)
}

internal actual fun createMultipartBody(parts: List<MultipartPart>, type: String, boundary: String): KmpRequestBody =
    MultipartBody.Builder(boundary).setType(type.toKmpMediaType()).apply {
        parts.forEach { addFormDataPart(it.name, it.filename, it.body) }
    }.build()

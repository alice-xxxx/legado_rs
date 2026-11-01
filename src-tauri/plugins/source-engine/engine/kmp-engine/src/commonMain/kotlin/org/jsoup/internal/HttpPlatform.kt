package org.jsoup.internal

import io.legado.app.help.http.KmpHttpClient
import io.legado.app.help.http.KmpRequestBody
import io.legado.app.help.http.toKmpRequestBody
import okio.IOException
import org.jsoup.Connection

internal fun urlEncodeForm(value: String, charset: String): String =
    io.legado.app.utils.formUrlEncode(value, charset)

internal expect fun decompressBody(bytes: ByteArray, contentEncoding: String?): ByteArray

internal expect fun buildPlatformClient(
    request: HttpConnectionRequest,
    shared: KmpHttpClient?
): KmpHttpClient

internal fun buildMultipartBody(data: Collection<Connection.KeyVal>, boundary: String): KmpRequestBody =
    io.legado.app.help.http.createMultipartBody(data.map { value ->
        val stream = value.inputStream()
        io.legado.app.help.http.MultipartPart(value.key(), if (stream != null) value.value() else null,
            if (stream != null) io.legado.app.help.http.streamMultipartBody(stream, value.contentType() ?: "application/octet-stream")
            else value.value().toKmpRequestBody(null))
    }, "multipart/form-data", boundary)

internal expect fun defaultSharedHttpClient(): KmpHttpClient?

internal expect fun uncheckedIoException(e: IOException): Throwable

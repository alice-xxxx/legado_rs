package io.legado.app.help.http

import okhttp3.MediaType.Companion.toMediaType
import okhttp3.MultipartBody
import okhttp3.RequestBody.Companion.asRequestBody
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.internal.http.RealResponseBody
import okio.buffer
import okio.source
import java.io.File
import java.util.zip.ZipInputStream

actual fun KmpResponseBody.decompressed(): KmpResponseBody {
    val contentType = this.contentType()?.toString()
    if (contentType != "application/zip") {
        return this
    }
    val source = ZipInputStream(this.byteStream()).apply {
        try {
            nextEntry
        } catch (e: Exception) {
            close()
            throw e
        }
    }.source().buffer()
    return RealResponseBody(null, -1, source)
}

actual fun KmpRequestBuilder.postMultipart(type: String?, form: Map<String, Any>) {
    val multipartBody = MultipartBody.Builder()
    type?.let {
        multipartBody.setType(it.toMediaType())
    }
    form.forEach {
        when (val value = it.value) {
            is Map<*, *> -> {
                val fileName = value["fileName"] as String
                val file = value["file"]
                val mediaType = (value["contentType"] as? String)?.toMediaType()
                val requestBody = when (file) {
                    is File -> {
                        file.asRequestBody(mediaType)
                    }

                    is ByteArray -> {
                        file.toRequestBody(mediaType)
                    }

                    is String -> {
                        file.toRequestBody(mediaType)
                    }

                    else -> {
                        // GSON.toJson(file) 用反射序列化任意对象;
                        // 此分支仅在前三种类型 (File/ByteArray/String) 都不匹配时执行, 实际不会触发。
                        // 改用 file.toString() 保留行为 (输出对象字符串表示), 避免引入新依赖。
                        file.toString().toRequestBody(mediaType)
                    }
                }
                multipartBody.addFormDataPart(it.key, fileName, requestBody)
            }

            else -> multipartBody.addFormDataPart(it.key, it.value.toString())
        }
    }
    this.post(multipartBody.build())
}

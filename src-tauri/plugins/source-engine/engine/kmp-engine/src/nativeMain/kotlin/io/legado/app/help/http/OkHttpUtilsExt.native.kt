// 源平台来源：data/src/nativeMain/kotlin/io/legado/app/help/http/OkHttpUtilsExt.native.kt。此 actual 已复制进提取模块，构建不读取源平台目录。
package io.legado.app.help.http

import io.legado.app.utils.systemCurrentTimeMillis

actual fun KmpResponseBody.decompressed(): KmpResponseBody {
    // 检查 contentType 是否是 application/zip (与 jvmAndAndroidMain 判断一致)
    val contentType = this.contentType()?.toString()
    if (contentType != "application/zip") {
        return this
    }
    // nativeMain 端无 zip 解压能力, 直接返回原 body (与 jvmAndAndroidMain 行为不同, 但功能降级)
    // 调用方在 iOS/鸿蒙端不依赖此功能 (DecompressInterceptor stub 不会触发解压链)
    return this
}

/**
 * nativeMain 端 [postMultipart] 实现: 手动构造 multipart/form-data body。
 *
 * 与 okhttp3.MultipartBody.Builder 字节级对齐:
 * - boundary: "legado-native-boundary-${systemCurrentTimeMillis()}-${hashCode}" (避免与正文冲突)
 * - 普通字段: `--boundary\r\nContent-Disposition: form-data; name="key"\r\n\r\nvalue\r\n`
 * - 文件字段 (Map<*, *>): `--boundary\r\nContent-Disposition: form-data; name="key"; filename="fileName"\r\n
 *   Content-Type: contentType\r\n\r\n<bytes>\r\n`
 * - 结束: `--boundary--\r\n`
 *
 * type 入参对应 OkHttp MultipartBody 的整体 MediaType (如 "multipart/form-data"),
 * 实际不生效 (Ktor setBody 时统一用 multipart/form-data; boundary=...); 与 jvm 行为基本一致。
 */
actual fun KmpRequestBuilder.postMultipart(type: String?, form: Map<String, Any>) {
    // 用 systemCurrentTimeMillis() (posix clock_gettime 包装) 替代 jvm 的 System.currentTimeMillis()
    val boundary = "legado-native-boundary-${systemCurrentTimeMillis()}-${(form.hashCode() and 0xFFFFFF)}"
    val CRLF = "\r\n"
    val sb = StringBuilder()
    val byteParts: MutableList<ByteArray> = mutableListOf()

    form.forEach { (key, value) ->
        sb.append("--").append(boundary).append(CRLF)
        when (value) {
            is Map<*, *> -> {
                val fileName = value["fileName"] as? String ?: "file"
                val file = value["file"]
                val contentTypeStr = value["contentType"] as? String
                sb.append("Content-Disposition: form-data; name=\"").append(key)
                    .append("\"; filename=\"").append(fileName).append("\"").append(CRLF)
                if (contentTypeStr != null) {
                    sb.append("Content-Type: ").append(contentTypeStr).append(CRLF)
                }
                sb.append(CRLF)
                // 字段头先写入 (sb), 字段内容追加到 byteParts (二进制安全)
                val headerBytes = sb.toString().encodeToByteArray()
                sb.setLength(0)
                val contentBytes: ByteArray = when (file) {
                    is ByteArray -> file
                    is String -> file.encodeToByteArray()
                    else -> file?.toString()?.encodeToByteArray() ?: ByteArray(0)
                }
                // header + content + CRLF 一起作为一个 part
                byteParts.add(headerBytes + contentBytes + CRLF.encodeToByteArray())
            }
            else -> {
                sb.append("Content-Disposition: form-data; name=\"").append(key).append("\"").append(CRLF)
                sb.append(CRLF)
                sb.append(value.toString()).append(CRLF)
                byteParts.add(sb.toString().encodeToByteArray())
                sb.setLength(0)
            }
        }
    }
    // 结束 boundary
    sb.append("--").append(boundary).append("--").append(CRLF)
    val endBytes = sb.toString().encodeToByteArray()

    // 拼接所有 part: 顺序合并 byteParts + endBytes
    val totalLength = byteParts.sumOf { it.size } + endBytes.size
    val bodyBytes = ByteArray(totalLength)
    var offset = 0
    for (part in byteParts) {
        // copyInto 逐参等价 System.arraycopy(part, 0, bodyBytes, offset, part.size)
        part.copyInto(bodyBytes, destinationOffset = offset)
        offset += part.size
    }
    endBytes.copyInto(bodyBytes, destinationOffset = offset)

    // 用 commonMain 工厂方法构造 body (ios/ohos 各自 actual 内部用 NativeKmp*/OhosKmp* 包装),
    // contentType 设为 multipart/form-data; boundary=...
    val contentTypeStr = (type ?: "multipart/form-data") + "; boundary=$boundary"
    val requestBody = bodyBytes.toKmpRequestBody(contentTypeStr.toKmpMediaType())
    post(requestBody)
}

// region 内部辅助扩展函数
/**
 * 从 [KmpMediaType] 中解析 charset 名 (与 okhttp3.MediaType.charset() 的名字解析部分对齐)。
 * 为 commonMain [KmpMediaType.charsetName] expect 的 nativeMain actual (iOS/鸿蒙共用)。
 *
 * 输入示例: "text/html; charset=UTF-8" -> "UTF-8"
 * 无 charset 返回 null。
 */
actual fun KmpMediaType.charsetName(): String? {
    val value = this.toString()
    val idx = value.indexOf("charset=", ignoreCase = true)
    if (idx < 0) return null
    var charsetStr = value.substring(idx + 8).trim()
    // 截到分号或字符串结尾
    val end = charsetStr.indexOf(';')
    if (end > 0) {
        charsetStr = charsetStr.substring(0, end).trim()
    }
    // 去除可能的引号
    if (charsetStr.startsWith("\"") && charsetStr.endsWith("\"")) {
        charsetStr = charsetStr.substring(1, charsetStr.length - 1)
    }
    return charsetStr.takeIf { it.isNotEmpty() }
}

// 源平台来源：foundation/src/iosMain/kotlin/io/legado/app/utils/TextCharsetCodec.ios.kt。此 actual 已复制进提取模块，构建不读取源平台目录。
@file:OptIn(kotlinx.cinterop.ExperimentalForeignApi::class)

package io.legado.app.utils

import io.legado.app.exception.NoStackTraceException
import kotlinx.cinterop.addressOf
import kotlinx.cinterop.readBytes
import kotlinx.cinterop.usePinned
import platform.CoreFoundation.CFStringConvertEncodingToNSStringEncoding
import platform.CoreFoundation.kCFStringEncodingGB_18030_2000
import platform.Foundation.NSString
import platform.Foundation.create
import platform.Foundation.dataUsingEncoding

@OptIn(kotlinx.cinterop.BetaInteropApi::class)
internal actual fun platformDecodeGb18030(
    bytes: ByteArray, offset: Int, length: Int
): String {
    if (length <= 0) return ""
    val encoding = gb18030Encoding()
    bytes.usePinned { pinned ->
        // 固定大小的探测块可能截断 GB18030 字符，裁尾后补替换字符。
        for (trim in 0..3) {
            val len = length - trim
            if (len <= 0) break
            // 这个 as 编译器报 USELESS_CAST 是假警告: NSString.create 的声明类型是 NSString?,
            // 删掉转换就无法当 String? 返回 (K/N 的 NSString↔String 映射只骗过了警告检查)
            @Suppress("USELESS_CAST")
            val decoded = NSString.create(
                bytes = pinned.addressOf(offset),
                length = len.toULong(),
                encoding = encoding,
            ) as String?
            if (decoded != null) {
                return if (trim == 0) decoded else decoded + '�'
            }
        }
    }
    throw NoStackTraceException(
        "iOS 端 GB18030 解码失败, TXT 文件可能已损坏或编码识别有误"
    )
}

/**
 * iOS actual: `NSString.dataUsingEncoding(allowLossyConversion=true)` 编码。
 * lossy=true 对齐 JVM REPLACE 模式 (不可映射字符降级输出而非失败);
 * 可映射字符字节输出与解码方向同表, 保证 TextFileCore 解码→编码往返字节数一致。
 */
internal actual fun platformEncodeGb18030(str: String): ByteArray {
    if (str.isEmpty()) return ByteArray(0)
    @Suppress("CAST_NEVER_SUCCEEDS")
    val nsString = str as NSString
    val data = nsString.dataUsingEncoding(gb18030Encoding(), allowLossyConversion = true)
        ?: throw NoStackTraceException(
            "iOS 端 GB18030 编码失败, 内容含无法转换的字符"
        )
    val size = data.length.toInt()
    if (size == 0) return ByteArray(0)
    return data.bytes?.readBytes(size) ?: ByteArray(0)
}

private fun gb18030Encoding(): ULong =
    CFStringConvertEncodingToNSStringEncoding(kCFStringEncodingGB_18030_2000.toUInt())

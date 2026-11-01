// 源平台来源：data/src/nativeMain/kotlin/org/jsoup/internal/HttpPlatform.native.kt。此 actual 已复制进提取模块，构建不读取源平台目录。
package org.jsoup.internal

import io.legado.app.help.http.KmpHttpClient
import io.legado.app.help.http.KmpHttpClientBuilder
import io.legado.app.help.http.OkHttpClientProviders
import okio.IOException
import kotlin.time.Duration
import kotlin.time.Duration.Companion.milliseconds

internal actual fun buildPlatformClient(
    request: HttpConnectionRequest,
    shared: KmpHttpClient?,
): KmpHttpClient {
    val builder = (shared?.newBuilder() ?: KmpHttpClientBuilder())
        .readTimeout(request.timeout().toLong().milliseconds)
    if (shared != null) {
        // jvm 端清掉共享 client 的 callTimeout(15s); native 端 0 = 未配置 → 默认 15s (尽力对齐)
        builder.callTimeout(Duration.ZERO)
    }
    return builder.build()
}

internal actual fun defaultSharedHttpClient(): KmpHttpClient? =
    runCatching { OkHttpClientProviders.get().okHttpClient }.getOrNull()

internal actual fun uncheckedIoException(e: IOException): Throwable = e

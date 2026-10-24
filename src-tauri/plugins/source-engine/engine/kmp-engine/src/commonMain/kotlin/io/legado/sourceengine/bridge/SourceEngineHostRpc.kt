package io.legado.sourceengine.bridge

import kotlinx.coroutines.runBlocking

/**
 * 同步 Provider/JS 绑定调用 Rust Host 的薄层。解析入口在整个操作期间固定当前 host，
 * 因此缓存、变量和 JS 库下载不会另起一条平台网络或文件访问路径。
 */
object SourceEngineHostRpc {
    fun downloadText(url: String): String = runBlocking {
        val host = requireNotNull(SourceEngineHostRegistry.current()) {
            "Source-engine HTTP call has no active Rust host"
        }
        host.executeHttp(
            HostHttpRequest(
                url = url,
                headers = listOf(HostHeader("User-Agent", "Mozilla/5.0 LegadoSourceEngine")),
                maxRedirects = 20,
                acceptInvalidCerts = true,
            ),
        ).body.decodeToString()
    }
}

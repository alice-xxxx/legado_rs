@file:OptIn(kotlinx.cinterop.ExperimentalForeignApi::class)
package io.legado.sourceengine.bridge

@kotlin.native.concurrent.ThreadLocal
private var activeHost: SourceEngineHost? = null
internal actual fun sourceHostOnThread(): SourceEngineHost? = activeHost
internal actual fun setSourceHostOnThread(host: SourceEngineHost?) { activeHost = host }
internal actual fun sourceExecutionCancelled(id: String): Boolean =
    io.legado.sourceengine.rust.legado_source_host_execution_cancelled(id) != 0

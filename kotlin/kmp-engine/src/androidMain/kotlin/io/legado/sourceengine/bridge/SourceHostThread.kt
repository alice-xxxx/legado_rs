package io.legado.sourceengine.bridge

private val activeHost = ThreadLocal<SourceEngineHost?>()
internal actual fun sourceHostOnThread(): SourceEngineHost? = activeHost.get()
internal actual fun setSourceHostOnThread(host: SourceEngineHost?) { activeHost.set(host) }
internal actual fun sourceExecutionCancelled(id: String): Boolean =
    io.legado.sourceengine.android.rustSourceExecutionCancelled(id)

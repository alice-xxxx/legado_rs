package io.legado.sourceengine.host

private val local=ThreadLocal<SourceEngineHost?>()
internal actual fun currentHost():SourceEngineHost?=local.get()
internal actual fun setHost(host:SourceEngineHost?){local.set(host)}

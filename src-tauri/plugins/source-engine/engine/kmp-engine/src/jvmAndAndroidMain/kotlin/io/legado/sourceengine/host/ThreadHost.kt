package io.legado.sourceengine.host
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.asContextElement
import kotlin.coroutines.CoroutineContext

private val local=ThreadLocal<SourceEngineHost?>()
internal actual fun currentHost():SourceEngineHost?=local.get()
internal actual fun setHost(host:SourceEngineHost?){local.set(host)}
internal actual fun hostContext(host:SourceEngineHost,dispatcher:CoroutineDispatcher):CoroutineContext=dispatcher+local.asContextElement(host)

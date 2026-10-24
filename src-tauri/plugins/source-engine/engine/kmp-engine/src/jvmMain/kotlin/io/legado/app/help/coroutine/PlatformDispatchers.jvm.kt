package io.legado.app.help.coroutine

import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers

actual val mainDispatcher: CoroutineDispatcher get() = Dispatchers.Default

private var isDebug: Boolean = false

fun registerJvmDebugState(debug: Boolean) {
    isDebug = debug
}

actual fun Throwable.printStackTraceOnDebug() {
    if (isDebug) {
        printStackTrace()
    }
}

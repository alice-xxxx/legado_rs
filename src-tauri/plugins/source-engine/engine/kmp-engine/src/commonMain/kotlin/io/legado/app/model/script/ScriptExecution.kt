package io.legado.app.model.script

import kotlin.coroutines.CoroutineContext

/** Restore caller state after completion, recursion errors or callback failures. */
internal inline fun <T> JsScope.withScriptExecution(context: CoroutineContext?, block: () -> T): T {
    val previousContext = coroutineContext
    val previousAllowed = allowScriptRun
    if (context != null) coroutineContext = context
    allowScriptRun = true
    recursiveCount++
    try {
        checkRecursive()
        return block()
    } finally {
        coroutineContext = previousContext
        allowScriptRun = previousAllowed
        recursiveCount--
    }
}

@PublishedApi
internal inline fun <T> JsScope?.withCoroutineContext(context: CoroutineContext, block: () -> T): T {
    val previous = this?.coroutineContext
    this?.coroutineContext = context
    try {
        return block()
    } finally {
        this?.coroutineContext = previous
    }
}

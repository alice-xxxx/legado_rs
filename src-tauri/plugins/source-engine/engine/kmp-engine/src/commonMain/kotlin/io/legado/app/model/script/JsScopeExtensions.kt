package io.legado.app.model.script

import kotlin.coroutines.CoroutineContext

val jsContext: JsScope
    get() = JsEngines.jsContext

val jsContextOrNull: JsScope?
    get() = JsEngines.jsContextOrNull

fun <T> runScriptWithContext(context: CoroutineContext, block: () -> T): T =
    JsEngines.runScriptWithContext(context, block)

suspend fun <T> runScriptWithContext(block: () -> T): T =
    JsEngines.runScriptWithContext(block)

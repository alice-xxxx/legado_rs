package com.script.quickjs

/** Engine bindings; source bindings add their default host objects. */
open class ScriptBindings : MutableMap<String, Any?> by LinkedHashMap() {
    var dangerousApi: Boolean = false
}

fun buildScriptBindings(block: (ScriptBindings) -> Unit): ScriptBindings = ScriptBindings().apply(block)

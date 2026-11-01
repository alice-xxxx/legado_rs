package com.script.jsdispatch

import org.json.JSONException
import org.json.JSONObject
import java.lang.reflect.InvocationTargetException

fun JsCoerce.toMapArg(v: Any?): Any? = when {
    v == null -> null
    v is Map<*, *> -> v
    v is String -> jsonStringToMap(v)
        ?: throw IllegalArgumentException("argument type mismatch")

    else -> throw IllegalArgumentException("argument type mismatch")
}

internal fun jsonStringToMap(value: String): Map<String, String>? {
    val trimmed = value.trim()
    if (!trimmed.startsWith("{")) return null
    return try {
        val json = JSONObject(trimmed)
        val map = LinkedHashMap<String, String>(json.length())
        json.keys().forEach { key -> map[key] = json.get(key).toString() }
        map
    } catch (_: JSONException) {
        null
    }
}

inline fun JsCoerce.invokeTarget(block: () -> Any?): Any? {
    try {
        return block()
    } catch (t: Throwable) {
        throw InvocationTargetException(t)
    }
}

fun JsCoerce.scoreStringArray(v: Any?): Int = when {
    v == null -> 0
    v is Array<*> && v.isArrayOf<String>() -> 0
    else -> 3
}


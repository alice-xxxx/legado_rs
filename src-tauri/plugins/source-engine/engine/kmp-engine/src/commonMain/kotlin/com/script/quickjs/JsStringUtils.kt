package com.script.quickjs

import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json

internal object JsStringUtils {
    fun escape(value: String): String = Json.encodeToString(value)

    fun primitive(value: Any?): String? = when (value) {
        null -> "null"
        is String -> escape(value)
        is Char -> escape(value.toString())
        is Number, is Boolean -> value.toString()
        else -> null
    }
}

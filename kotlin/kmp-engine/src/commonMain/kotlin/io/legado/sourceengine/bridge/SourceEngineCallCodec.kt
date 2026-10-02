package io.legado.sourceengine.bridge

import io.legado.app.utils.KS_JSON
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonPrimitive

/** Decode the stable Rust wire shape once, shared by the JVM and native mobile entry points. */
internal fun decodeSourceEngineCall(requestJson: String): SourceEngineCall {
    val request = KS_JSON.parseToJsonElement(requestJson) as? JsonObject
        ?: error("Source-engine request must be a JSON object")

    fun required(name: String) = request[name]
        ?: error("Missing required source-engine field: $name")

    fun requiredString(name: String) = required(name).jsonPrimitive.contentOrNull
        ?: error("Source-engine field '$name' must be a string")

    return SourceEngineCall(
        operation = requiredString("operation"),
        sourceJson = required("source").toString(),
        keyword = request["keyword"]?.jsonPrimitive?.contentOrNull,
        page = request["page"]?.jsonPrimitive?.intOrNull,
        bookJson = request["book"]?.takeUnless { it == JsonNull }?.toString(),
        chapterJson = request["chapter"]?.takeUnless { it == JsonNull }?.toString(),
        nextChapterUrl = request["nextChapterUrl"]?.jsonPrimitive?.contentOrNull,
    )
}

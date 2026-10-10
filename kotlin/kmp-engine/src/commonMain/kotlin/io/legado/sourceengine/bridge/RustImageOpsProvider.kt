package io.legado.sourceengine.bridge

import io.legado.app.help.image.ImageOps
import io.legado.app.help.image.ImageRef
import io.legado.app.utils.Base64Lenient
import io.legado.app.utils.KS_JSON
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonObjectBuilder
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put

/**
 * KMP adapter for Rust's mature pixel codecs/operations.
 *
 * ImageRef owns a regular Kotlin ByteArray containing a normalized PNG. Each
 * synchronous request is stateless, so no native handle needs GC/finalizer
 * coordination and JS rules keep the same `image.*` API on every host.
 */
class RustImageOpsProvider(private val request: (String) -> String) : ImageOps {
    private companion object {
        const val MAX_STITCH_BYTES = 64L * 1024 * 1024
    }

    private class PngImageRef(val bytes: ByteArray) : ImageRef

    override fun decode(bytes: ByteArray): ImageRef = decodeBytes(bytes)

    override fun decode(base64: String): ImageRef {
        val payload = base64.substringAfter("base64,", base64)
        return decodeBytes(Base64Lenient.decode(payload))
    }

    private fun decodeBytes(bytes: ByteArray): ImageRef {
        val value = call("decode") {
            put("bytesBase64", Base64Lenient.encodeToString(bytes))
        }
        return PngImageRef(decodeRequiredBytes(value, "pngBase64"))
    }

    override fun encode(img: ImageRef, format: String, quality: Int): ByteArray {
        val value = call("encode") {
            putImage(img)
            put("format", format)
            put("quality", quality)
        }
        return decodeRequiredBytes(value, "bytesBase64")
    }

    override fun split(img: ImageRef, rows: Int, cols: Int): List<ImageRef> {
        val value = call("split") {
            putImage(img)
            put("rows", rows)
            put("cols", cols)
        }
        return value.getValue("imagesBase64").jsonArray.map { encoded ->
            PngImageRef(Base64Lenient.decode(encoded.jsonPrimitive.content))
        }
    }

    override fun stitch(imgs: List<ImageRef>, direction: String): ImageRef {
        require(imgs.size <= 32) { "image.stitch supports at most 32 images" }
        val images = imgs.map(::imageOf)
        val byteCount = images.sumOf { it.bytes.size.toLong() }
        require(byteCount <= MAX_STITCH_BYTES) {
            "image.stitch input exceeds the 64 MiB transport limit"
        }
        val value = call("stitch") {
            put("imagesBase64", JsonArray(images.map {
                JsonPrimitive(Base64Lenient.encodeToString(it.bytes))
            }))
            put("direction", direction)
        }
        return PngImageRef(decodeRequiredBytes(value, "pngBase64"))
    }

    override fun crop(img: ImageRef, x: Int, y: Int, w: Int, h: Int): ImageRef {
        val value = call("crop") {
            putImage(img)
            put("x", x)
            put("y", y)
            put("w", w)
            put("h", h)
        }
        return PngImageRef(decodeRequiredBytes(value, "pngBase64"))
    }

    override fun rotate(img: ImageRef, deg: Int): ImageRef {
        val value = call("rotate") {
            putImage(img)
            put("deg", deg)
        }
        return PngImageRef(decodeRequiredBytes(value, "pngBase64"))
    }

    override fun flip(img: ImageRef, direction: String): ImageRef {
        val value = call("flip") {
            putImage(img)
            put("direction", direction)
        }
        return PngImageRef(decodeRequiredBytes(value, "pngBase64"))
    }

    override fun size(img: ImageRef): Map<String, Int> {
        val value = call("size") { putImage(img) }
        return mapOf(
            "w" to value.getValue("w").jsonPrimitive.content.toInt(),
            "h" to value.getValue("h").jsonPrimitive.content.toInt(),
        )
    }

    // Rust owns no native allocation: the complete image pixels remain inside
    // the Kotlin reference, so the standard scope behavior is sufficient.
    override fun <T> withScope(block: () -> T): T = block()

    private fun JsonObjectBuilder.putImage(image: ImageRef) {
        put("imageBase64", encodeImage(image))
    }

    private fun encodeImage(image: ImageRef): String = imageOf(image).let {
        Base64Lenient.encodeToString(it.bytes)
    }

    private fun imageOf(image: ImageRef): PngImageRef = image as? PngImageRef
        ?: throw IllegalArgumentException("image: argument is not a Rust image.* result")

    private fun decodeRequiredBytes(value: JsonObject, key: String): ByteArray {
        val encoded = value[key]?.jsonPrimitive?.content
            ?: throw IllegalStateException("Rust image operation returned no $key")
        return Base64Lenient.decode(encoded)
    }

    private fun call(operation: String, args: JsonObjectBuilder.() -> Unit): JsonObject {
        val wire = buildJsonObject {
            put("op", operation)
            args()
        }.toString()
        val envelope = try {
            KS_JSON.parseToJsonElement(request(wire)).jsonObject
        } catch (error: Throwable) {
            throw IllegalStateException("Rust image host returned invalid JSON", error)
        }
        if (envelope["ok"]?.jsonPrimitive?.content != "true") {
            val message = envelope["error"]?.jsonPrimitive?.content
                ?: "Rust image operation failed"
            throw IllegalArgumentException(message)
        }
        return envelope["value"]?.jsonObject
            ?: throw IllegalStateException("Rust image host returned no result")
    }
}

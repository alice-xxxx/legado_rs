package io.legado.app.model.script

import io.legado.app.help.JsCryptoProviders
import io.legado.app.utils.EncoderUtils
import io.legado.app.utils.toHexLower

/** Java 类签名的薄适配；加密、摘要、随机数仍由宿主的同一套实现完成。 */
internal object JavaCryptoCompat {
    val classes = listOf(
        "java.util.Base64", "android.util.Base64", "java.util.Arrays", "java.util.UUID",
        "java.security.MessageDigest", "java.security.SecureRandom", "javax.crypto.Cipher", "javax.crypto.Mac",
        "javax.crypto.spec.SecretKeySpec", "javax.crypto.spec.IvParameterSpec", "javax.crypto.spec.GCMParameterSpec"
    )
    sealed interface Value
    private data class Key(val data: ByteArray, val algorithm: String) : Value
    private data class Parameters(val iv: ByteArray, val tagBits: Int = 128) : Value
    private data class Codec(val flags: Int, val decoder: Boolean = false) : Value
    private data object Random : Value
    private class Digest(val algorithm: String, val mac: Boolean = false) : Value {
        val chunks = mutableListOf<ByteArray>()
        var key: ByteArray? = null
    }
    private class Cipher(val transformation: String) : Value {
        var mode = 0
        var key: Key? = null
        var parameters: Parameters? = null
    }

    fun construct(name: String, args: List<Any?>): Any = when (name) {
        "javax.crypto.spec.SecretKeySpec" -> Key(slice(args), args.last() as String)
        "javax.crypto.spec.IvParameterSpec" -> Parameters(slice(args))
        "javax.crypto.spec.GCMParameterSpec" -> Parameters(javaBytes(args[1]), args.int(0))
        "java.security.SecureRandom" -> Random
        else -> error("Unsupported Java constructor: $name")
    }

    fun field(name: String, field: String): Int? = when (name) {
        "javax.crypto.Cipher" -> when (field) { "ENCRYPT_MODE" -> 1; "DECRYPT_MODE" -> 2; else -> null }
        "android.util.Base64" -> mapOf("DEFAULT" to 0, "NO_PADDING" to 1, "NO_WRAP" to 2, "CRLF" to 4, "URL_SAFE" to 8, "NO_CLOSE" to 16)[field]
        else -> null
    }

    fun callStatic(name: String, method: String, args: List<Any?>): Any? = when (name) {
        "java.security.MessageDigest" -> { require(method == "getInstance"); Digest(args[0] as String) }
        "javax.crypto.Mac" -> { require(method == "getInstance"); Digest(args[0] as String, true) }
        "javax.crypto.Cipher" -> { require(method == "getInstance"); Cipher(args[0] as String) }
        "java.util.Base64" -> when (method) {
            "getEncoder" -> Codec(2)
            "getUrlEncoder" -> Codec(10)
            "getMimeEncoder" -> Codec(4)
            "getDecoder" -> Codec(2, true)
            "getUrlDecoder" -> Codec(10, true)
            "getMimeDecoder" -> Codec(4, true)
            else -> error("Unsupported Base64 method: $method")
        }
        "android.util.Base64" -> {
            val data = if (args.size == 4) javaBytes(args[0]).copyOfRange(args.int(1), args.int(1) + args.int(2)) else args[0]
            val flags = (args.last() as Number).toInt()
            when (method) {
                "decode" -> EncoderUtils.base64DecodeToByteArray(if (data is String) data else javaBytes(data).decodeToString(), flags)
                "encodeToString" -> EncoderUtils.base64Encode(javaBytes(data), flags)
                "encode" -> EncoderUtils.base64Encode(javaBytes(data), flags).encodeToByteArray()
                else -> error("Unsupported Android Base64 method: $method")
            }
        }
        "java.util.Arrays" -> when (method) {
            "copyOfRange" -> javaBytes(args[0]).let { bytes ->
                val from = args.int(1); val to = args.int(2)
                require(from in 0..bytes.size && to >= from)
                ByteArray(to - from).also { bytes.copyInto(it, 0, from, minOf(to, bytes.size)) }
            }
            "copyOf" -> javaBytes(args[0]).copyOf(args.int(1))
            "toString" -> javaBytes(args[0]).joinToString(", ", "[", "]")
            "equals" -> javaBytes(args[0]).contentEquals(javaBytes(args[1]))
            else -> error("Unsupported Arrays method: $method")
        }
        "java.util.UUID" -> {
            require(method == "randomUUID")
            val bytes = JsCryptoProviders.get().randomBytes(16)
            bytes[6] = ((bytes[6].toInt() and 15) or 64).toByte()
            bytes[8] = ((bytes[8].toInt() and 63) or 128).toByte()
            val hex = bytes.toHexLower()
            listOf(hex.take(8), hex.substring(8,12), hex.substring(12,16), hex.substring(16,20), hex.substring(20)).joinToString("-")
        }
        else -> error("Unsupported Java static method: $name.$method")
    }

    fun call(value: Value, method: String, args: List<Any?>): Any? = when (value) {
        is Key -> when (method) { "getEncoded" -> value.data.copyOf(); "getAlgorithm" -> value.algorithm; else -> error("Unsupported key method: $method") }
        is Parameters -> when (method) { "getIV" -> value.iv.copyOf(); "getTLen" -> value.tagBits; else -> error("Unsupported parameter method: $method") }
        is Codec -> when (method) {
            "withoutPadding" -> { require(!value.decoder); value.copy(flags = value.flags or 1) }
            "encodeToString", "encode" -> {
                require(!value.decoder) { "Base64 decoder cannot encode" }
                val encoded = EncoderUtils.base64Encode(javaBytes(args[0]), value.flags).removeSuffix("\r\n")
                if (method == "encode") encoded.encodeToByteArray() else encoded
            }
            "decode" -> {
                require(value.decoder) { "Base64 encoder cannot decode" }
                val text = if (args[0] is String) args[0] as String else javaBytes(args[0]).decodeToString()
                if (value.flags and 4 == 0) {
                    val alphabet = if (value.flags and 8 == 0) "+/=" else "-_="
                    require(text.all { it in 'A'..'Z' || it in 'a'..'z' || it in '0'..'9' || it in alphabet }) { "Invalid Base64 character" }
                }
                EncoderUtils.base64DecodeToByteArray(text, value.flags)
            }
            else -> error("Unsupported Base64 codec method: $method")
        }
        is Digest -> when (method) {
            "init" -> { require(value.mac); value.key = (args[0] as Key).data; value.chunks.clear(); null }
            "update" -> { value.chunks.add(slice(args)); null }
            "reset" -> { value.chunks.clear(); null }
            "digest", "doFinal" -> {
                require((method == "doFinal") == value.mac)
                if (args.isNotEmpty()) value.chunks.add(slice(args))
                val data = ByteArray(value.chunks.sumOf { it.size })
                var offset = 0
                value.chunks.forEach { it.copyInto(data, offset); offset += it.size }
                value.chunks.clear()
                if (value.mac) JsCryptoProviders.get().hmacBytes(value.algorithm, value.key ?: error("Mac is not initialized"), data)
                else JsCryptoProviders.get().digestBytes(value.algorithm, data)
            }
            else -> error("Unsupported digest method: $method")
        }
        is Cipher -> when (method) {
            "init" -> {
                require(args.int(0) in 1..2) { "Unsupported Cipher mode" }
                value.mode = args.int(0); value.key = args[1] as Key
                value.parameters = args.getOrNull(2) as? Parameters
                require(value.parameters?.tagBits == null || value.parameters?.tagBits == 128) { "Only 128-bit GCM tags are supported" }
                null
            }
            "doFinal" -> {
                val key = value.key ?: error("Cipher is not initialized")
                val crypto = JsCryptoProviders.get().createSymmetricCrypto(value.transformation, key.data, value.parameters?.iv)
                val data = if (args.isEmpty()) ByteArray(0) else slice(args)
                if (value.mode == 1) crypto.encrypt(data) else crypto.decrypt(data.toHexLower())
            }
            "getIV" -> value.parameters?.iv?.copyOf()
            "getAlgorithm" -> value.transformation
            else -> error("Unsupported Cipher method: $method")
        }
        Random -> when (method) {
            "generateSeed" -> JsCryptoProviders.get().randomBytes(args.int(0))
            else -> error("Unsupported SecureRandom method: $method")
        }
    }

    private fun List<Any?>.int(index: Int) = (get(index) as Number).toInt()
    private fun slice(args: List<Any?>): ByteArray {
        val bytes = javaBytes(args[0])
        return if (args.size >= 3 && args[1] is Number && args[2] is Number) bytes.copyOfRange(args.int(1), args.int(1) + args.int(2)) else bytes.copyOf()
    }
}

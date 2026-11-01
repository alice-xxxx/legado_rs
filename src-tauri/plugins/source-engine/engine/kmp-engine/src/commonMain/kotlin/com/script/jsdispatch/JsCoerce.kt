package com.script.jsdispatch

import kotlin.jvm.JvmStatic

object JsCoerce {

    @JvmStatic
    fun toInt(v: Any?): Int = if (v == null) 0 else (v as Number).toInt()

    @JvmStatic
    fun toLong(v: Any?): Long = if (v == null) 0L else (v as Number).toLong()

    @JvmStatic
    fun toShort(v: Any?): Short = if (v == null) 0 else (v as Number).toShort()

    @JvmStatic
    fun toByte(v: Any?): Byte = if (v == null) 0 else (v as Number).toByte()

    @JvmStatic
    fun toFloat(v: Any?): Float = if (v == null) 0f else (v as Number).toFloat()

    @JvmStatic
    fun toDouble(v: Any?): Double = if (v == null) 0.0 else (v as Number).toDouble()

    @JvmStatic
    fun toIntOrNull(v: Any?): Int? = if (v == null) null else (v as Number).toInt()

    @JvmStatic
    fun toLongOrNull(v: Any?): Long? = if (v == null) null else (v as Number).toLong()

    @JvmStatic
    fun toShortOrNull(v: Any?): Short? = if (v == null) null else (v as Number).toShort()

    @JvmStatic
    fun toByteOrNull(v: Any?): Byte? = if (v == null) null else (v as Number).toByte()

    @JvmStatic
    fun toFloatOrNull(v: Any?): Float? = if (v == null) null else (v as Number).toFloat()

    @JvmStatic
    fun toDoubleOrNull(v: Any?): Double? = if (v == null) null else (v as Number).toDouble()

    @JvmStatic
    fun toBooleanArg(v: Any?): Boolean = when (v) {
        null -> throw IllegalArgumentException("argument type mismatch")
        else -> v as Boolean
    }

    @JvmStatic
    fun toBooleanOrNull(v: Any?): Boolean? = when (v) {
        null -> null
        is Boolean -> v
        else -> throw IllegalArgumentException("argument type mismatch")
    }

    @JvmStatic
    fun toCharArg(v: Any?): Char = when (v) {
        null -> throw IllegalArgumentException("argument type mismatch")
        else -> v.toString().firstOrNull() ?: ' '
    }

    @JvmStatic
    fun toCharOrNull(v: Any?): Char? = when (v) {
        null -> null
        is Char -> v
        else -> throw IllegalArgumentException("argument type mismatch")
    }

    @JvmStatic
    fun refArg(v: Any?, ok: Boolean): Any? =
        if (v == null || ok) v else throw IllegalArgumentException("argument type mismatch")

    @JvmStatic
    fun isArrayLike(v: Any?): Boolean =
        v == null || v is ByteArray || v is List<*> || v is Array<*> ||
            v is IntArray || v is LongArray || v is ShortArray || v is CharArray ||
            v is FloatArray || v is DoubleArray || v is BooleanArray

    @JvmStatic
    fun isByteArrayStrict(v: Any?): Boolean = when (v) {
        null, is ByteArray -> true
        is List<*> -> v.all { it is Number }
        is Array<*> -> v.all { it is Number }
        is IntArray, is LongArray, is ShortArray, is FloatArray, is DoubleArray -> true
        else -> false
    }

    @JvmStatic
    fun isStringArrayStrict(v: Any?): Boolean = when (v) {
        null -> true
        is List<*> -> v.none { it is Map<*, *> }
        is Array<*> -> v.none { it is Map<*, *> }
        else -> isArrayLike(v)
    }

    @JvmStatic
    fun toByteArrayArg(v: Any?): ByteArray? = when (v) {
        null -> null
        is ByteArray -> v
        is List<*> -> ByteArray(v.size) { i -> (v[i] as Number).toByte() }
        is Array<*> -> ByteArray(v.size) { i -> (v[i] as Number).toByte() }
        is IntArray -> ByteArray(v.size) { i -> v[i].toByte() }
        is LongArray -> ByteArray(v.size) { i -> v[i].toByte() }
        is ShortArray -> ByteArray(v.size) { i -> v[i].toByte() }
        is FloatArray -> ByteArray(v.size) { i -> v[i].toInt().toByte() }
        is DoubleArray -> ByteArray(v.size) { i -> v[i].toInt().toByte() }
        else -> throw IllegalArgumentException("argument type mismatch")
    }

    @JvmStatic
    fun toStringArrayArg(v: Any?): Array<String?>? = when (v) {
        null -> null
        is List<*> -> Array(v.size) { i -> v[i]?.toString() }
        is Array<*> -> Array(v.size) { i -> v[i]?.toString() }
        is IntArray -> Array<String?>(v.size) { i -> v[i].toString() }
        is LongArray -> Array<String?>(v.size) { i -> v[i].toString() }
        is ShortArray -> Array<String?>(v.size) { i -> v[i].toString() }
        is CharArray -> Array<String?>(v.size) { i -> v[i].toString() }
        is FloatArray -> Array<String?>(v.size) { i -> v[i].toString() }
        is DoubleArray -> Array<String?>(v.size) { i -> v[i].toString() }
        is BooleanArray -> Array<String?>(v.size) { i -> v[i].toString() }
        is ByteArray -> Array<String?>(v.size) { i -> v[i].toString() }
        else -> throw IllegalArgumentException("argument type mismatch")
    }

    @JvmStatic
    fun isIntegerNumber(n: Number): Boolean = when (n) {
        is Int, is Long, is Short, is Byte -> true
        is Double -> n == n.toInt().toDouble()
        is Float -> n == n.toInt().toFloat()
        else -> false
    }

    @JvmStatic
    fun scoreIntPrim(v: Any?): Int = when {
        v is Int -> 0
        v is Number -> if (isIntegerNumber(v)) 2 else 4
        else -> 5
    }

    @JvmStatic
    fun scoreLongPrim(v: Any?): Int = when {
        v is Long -> 0
        v is Number -> if (isIntegerNumber(v)) 2 else 4
        else -> 5
    }

    @JvmStatic
    fun scoreShortPrim(v: Any?): Int = if (v is Short) 0 else 5

    @JvmStatic
    fun scoreBytePrim(v: Any?): Int = if (v is Byte) 0 else 5

    @JvmStatic
    fun scoreFloatPrim(v: Any?): Int = when {
        v is Float -> 0
        v is Number -> if (isIntegerNumber(v)) 3 else 2
        else -> 5
    }

    @JvmStatic
    fun scoreDoublePrim(v: Any?): Int = when {
        v is Double -> 0
        v is Number -> if (isIntegerNumber(v)) 3 else 2
        else -> 5
    }

    @JvmStatic
    fun scoreRef(v: Any?, k: kotlin.reflect.KClass<*>): Int = when {
        v == null -> 0
        v::class == k -> 0
        k.isInstance(v) -> 1
        else -> 5
    }

    @JvmStatic
    fun scoreByteArray(v: Any?): Int = when {
        v == null -> 0
        v is ByteArray -> 0
        else -> 3
    }

    }

@file:OptIn(kotlinx.cinterop.ExperimentalForeignApi::class)

package io.legado.app.model.script

import io.legado.app.napi.quickjs.*
import kotlinx.cinterop.*

/** 与 JNI 一样按 UTF-16 单元传输，保留 NUL、补充字符和孤立代理项。 */
internal fun nativeJsString(ctx: CPointer<JSContext>, text: String): CValue<JSValue> = memScoped {
    val chars = allocArray<UShortVar>(maxOf(1, text.length))
    for (i in text.indices) chars[i] = text[i].code.toUShort()
    JS_NewStringUTF16(ctx, chars, text.length.toULong())
}

internal fun nativeStringOrNull(ctx: CPointer<JSContext>, value: CValue<JSValue>): String? = memScoped {
    val length = alloc<ULongVar>()
    val chars = JS_ToCStringLenUTF16(ctx, length.ptr, value) ?: return@memScoped null
    try {
        require(length.value <= Int.MAX_VALUE.toULong()) { "JS string is too large" }
        CharArray(length.value.toInt()) { chars[it].toInt().toChar() }.concatToString()
    } finally {
        JS_FreeCStringUTF16(ctx, chars)
    }
}

/** JS_SetProperty 消费 value；用 atom 保留属性名中的 NUL。 */
internal fun nativeSetProperty(ctx: CPointer<JSContext>, obj: CValue<JSValue>, key: String, value: CValue<JSValue>) {
    val name = nativeJsString(ctx, key)
    if (qjs_IsException(name) != 0) {
        JS_FreeValue(ctx, value)
        throw NativeScriptException("Cannot allocate JS property name")
    }
    val atom = try { JS_ValueToAtom(ctx, name) } finally { JS_FreeValue(ctx, name) }
    if (atom == 0u) {
        JS_FreeValue(ctx, value)
        throw NativeScriptException("Cannot allocate JS property")
    }
    try {
        if (JS_SetProperty(ctx, obj, atom, value) < 0) throw NativeScriptException("Cannot set JS property")
    } finally {
        JS_FreeAtom(ctx, atom)
    }
}

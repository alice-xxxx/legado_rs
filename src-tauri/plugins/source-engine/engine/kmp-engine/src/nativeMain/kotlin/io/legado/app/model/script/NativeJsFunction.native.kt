@file:OptIn(kotlinx.cinterop.ExperimentalForeignApi::class)

package io.legado.app.model.script

import io.legado.app.napi.quickjs.*
import kotlinx.cinterop.*
import platform.posix.memcpy

/** 只在所属 scope 的执行线程内同步回调；JS 引用随 context 一同释放。 */
internal class NativeJsFunction private constructor(
    private val owner: NativeJsScope,
    private val index: Int
) : JsFn, JavaClassCompat.Callback {
    override val dangerousApi: Boolean get() = owner.dangerousApi

    fun toJsValue(ctx: CPointer<JSContext>): CValue<JSValue> {
        check(NativeJsEngine.currentScope() === owner && owner.ctx == ctx) { "Source callback context was released or belongs to another thread" }
        return JS_DupValue(ctx, owner.functions[index])
    }

    override fun call(method: String, args: List<Any?>): Any? = call(*args.toTypedArray())

    override fun call(vararg args: Any?): Any? {
        val ctx = checkNotNull(owner.ctx) { "Source callback context was released" }
        check(NativeJsEngine.currentScope() === owner) { "Source callback requires its active JS scope" }
        owner.ensureActive()
        owner.recursiveCount++
        try {
            owner.checkRecursive()
            val function = toJsValue(ctx)
            val values = ArrayList<CValue<JSValue>>(args.size)
            try {
                if (qjs_IsException(function) != 0) NativeJsEngine.fromJsValue(ctx, function)
                for (arg in args) {
                    val value = NativeJsEngine.toJsValue(ctx, arg) ?: error("Unsupported callback argument")
                    values.add(value)
                    if (qjs_IsException(value) != 0) NativeJsEngine.fromJsValue(ctx, value)
                }
                return memScoped {
                    val argv = if (values.isEmpty()) null else allocArray<JSValue>(values.size)
                    for (i in values.indices) values[i].useContents {
                        memcpy(interpretCPointer<ByteVar>(argv!![i.toLong()].rawPtr), interpretCPointer<ByteVar>(rawPtr), sizeOf<JSValue>().toULong())
                    }
                    val result = JS_Call(ctx, function, cValue { tag = JS_TAG_UNDEFINED.toLong() }, values.size, argv)
                    try { NativeJsEngine.fromJsValue(ctx, result) } finally { JS_FreeValue(ctx, result) }
                }
            } finally {
                values.forEach { JS_FreeValue(ctx, it) }
                JS_FreeValue(ctx, function)
            }
        } finally {
            owner.recursiveCount--
        }
    }

    companion object {
        fun capture(ctx: CPointer<JSContext>, value: CValue<JSValue>): NativeJsFunction {
            val owner = checkNotNull(NativeJsEngine.currentScope()) { "JS function requires an active scope" }
            check(owner.ctx == ctx)
            val retained = JS_DupValue(ctx, value)
            try {
                owner.functions.add(retained)
            } catch (error: Throwable) {
                JS_FreeValue(ctx, retained)
                throw error
            }
            return NativeJsFunction(owner, owner.functions.lastIndex)
        }
    }
}

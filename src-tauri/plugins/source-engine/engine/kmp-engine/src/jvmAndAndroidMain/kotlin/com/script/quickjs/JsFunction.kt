package com.script.quickjs

/**
 * JS function 的 Java 侧包装。
 *
 * 对应 rhino 的 `org.mozilla.javascript.Function`。
 * 用于需要把 JS function
 * 作为对象传递/调用的场景。
 *
 * 架构 A (自封装 native QuickJS):
 * - 持有 [context] (QuickJsContext), 通过 [QuickJsNative.nativeEval] 执行 JS
 * - 不再依赖 quickjs-kt 的 QuickJs (已移除)
 *
 * [call] 仅在当前线程持有活跃的 [context] 时同步执行。
 */
class JsFunction(
    val context: QuickJsContext,
    val functionExpr: String,
    val dangerousApi: Boolean = false
) {
    /**
     * native JSContext 指针 (取自 [context])。
     */
    val ctxPtr get() = context.ctxPtr

    /**
     * 调用 JS function。
     *
     * @param args 参数,基本类型直接拼字面量,Java 对象通过 __wrapJavaHandle(handle) 句柄包装
     * @return JS function 返回值
     *   - 基本类型 (String/Number/Boolean) 直接返回
     *   - Java 对象由 native 层 toJavaObject 解包为原始 Java 对象返回
     *   - null/undefined 返回 null
     */
    fun call(vararg args: Any?): Any? = try {
        check(QuickJsContext.threadLocalContext.get() === context && QuickJsContext.findByCtxPtr(ctxPtr) === context) {
            "Source callback context was released or belongs to another thread"
        }
        val jsArgs = args.joinToString(",") { argToJsExpr(it) }
        context.dangerousApi = dangerousApi
        if (context.lastSyncedDangerousApi != dangerousApi) {
            QuickJsNative.nativeSetDangerousApi(ctxPtr, dangerousApi)
            context.lastSyncedDangerousApi = dangerousApi
        }
        QuickJsNative.nativeEval(ctxPtr, "($functionExpr)($jsArgs)")
    } catch (error: Throwable) {
        logQuickJsWarn(TAG, "call failed: $functionExpr", error)
        null
    }

    /**
     * 把 Java 参数转为 JS 表达式字符串。
     *
     * - null/基本类型直接拼字面量
     * - Java 对象通过 [JavaObjectBridge.registerObject] 注册句柄,
     *   用 __wrapJavaHandle(handle) 包装为 JS JavaObject (走 native trap)
     */
    private fun argToJsExpr(arg: Any?): String {
        if (arg == null) return "null"
        if (arg is String) return JsStringUtils.escape(arg)
        if (arg is Number) return arg.toString()
        if (arg is Boolean) return arg.toString()
        if (arg is Char) return JsStringUtils.escape(arg.toString())
        // Java 对象通过句柄包装
        if (!JsSecurityPolicy.isObjectVisible(arg, dangerousApi)) return "null"
        val handle = JavaObjectBridge.registerObject(arg)
        // __wrapJavaHandle 是 native binding, 返回 JavaObject
        // 注意: JS 数字字面量不能有 L 后缀(那是 Java/Kotlin Long 语法),会被 JS 解析为 SyntaxError
        return "__wrapJavaHandle($handle)"
    }

    companion object {
        private const val TAG = "JsFunction"
    }
}

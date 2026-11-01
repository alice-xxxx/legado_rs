package com.script.quickjs

import com.script.quickjs.JsFunctionHandle.Companion.releaseScope
import java.util.concurrent.atomic.AtomicLong

/**
 * JS function 对象的句柄包装。
 *
 * 用于 JavaAdapter 回调:Java 侧触发接口方法时,通过句柄找到 JS function 对象,
 * 在对应 native QuickJs ctx 上 evaluate 调用。
 *
 * 架构 A (native trap + 极简 bootstrap):
 * - 持有 [ctxPtr] (native JSContext 指针), 通过 [QuickJsNative.nativeEval] 执行 JS
 * - 不再依赖 quickjs-kt 的 QuickJs (已移除)
 *
 * 线程模型说明:
 * - native JSRuntime 单线程, JS 执行必须在持有 ctx 的线程
 * - 如果 JavaAdapter 回调发生在 JS evaluate 进行中 (JS->Kotlin->JavaAdapter->JS),
 *   会重入 nativeEval。native JSRuntime 是可重入的 (JS_RunEval 内部支持重入)
 * - 常见场景 (OnClickListener) 是 evaluate 完成后回调, 不会重入
 *
 * JS function 通过 [jsObjectExpr] 引用, 在 QuickJs 全局作用域里是唯一变量名。
 *
 * 资源管理: 句柄按 [QuickJsContext.scopeId] 分组, [releaseScope] 释放某 scope 全部句柄,
 * 由 [QuickJsContext.close] 触发。避免长期运行累积内存泄漏。
 *
 * 注意: 不会释放 [ctxPtr] 本身 (由 [QuickJsContext.close] 释放)。
 * 这里只清理 handleMap 中的 JsFunctionHandle 条目, 让强引用断开。
 * JS 全局变量 (__jsFn_xxx__) 的清理依赖 ctx 销毁时的 GC。
 */
class JsFunctionHandle private constructor(
    val ctxPtr: Long,
    val jsObjectExpr: String,
    val dangerousApi: Boolean
) {
    /**
     * 调用 JS function 对象上的方法。
     *
     * @param methodName JS 对象的方法名 (如 "onClick")
     * @param args Java 方法参数, 会通过 [javaArgToJsExpr] 转换为 JS 表达式
     * @return JS function 返回值 (基本类型 / Java 对象包装为 JavaObject / null)
     */
    fun invokeJsMethod(methodName: String, args: Array<Any?>): Any? = try {
        val owner = findOwnerCtx()
        check(owner != null && QuickJsContext.threadLocalContext.get() === owner) {
            "Source callback context was released or belongs to another thread"
        }
        val jsArgs = args.joinToString(",") { scriptArgument(it, dangerousApi) }
        QuickJsNative.nativeEval(ctxPtr,
            "(function(){var impl=$jsObjectExpr;return typeof impl==='function'?impl($jsArgs):(impl && impl.$methodName && impl.$methodName($jsArgs));})();")
    } catch (error: Throwable) {
        logQuickJsWarn(TAG, "invokeJsMethod failed: $jsObjectExpr.$methodName", error)
        null
    }


    /**
     * 在 ctxPtrToCtxMap 中查找当前 ctxPtr 对应的 QuickJsContext。
     * 用于确认 JavaAdapter 回调仍在持有此上下文的线程中。
     */
    private fun findOwnerCtx(): QuickJsContext? {
        // 简单实现: 遍历所有活跃 scope 查找
        // 优化: 可以维护一个 ctxPtr -> QuickJsContext 的全局映射, 但当前 scope 数量少, 直接遍历
        return QuickJsContext.findByCtxPtr(ctxPtr)
    }

    companion object {
        private const val TAG = "JsFunctionHandle"

        private val handleCounter = AtomicLong(1L)
        private val handleMap = HashMap<Long, JsFunctionHandle>()

        /** scopeId -> 该 scope 注册的 JsFunctionHandle 句柄集合, close 时批量释放 */
        private val scopeHandles = HashMap<Long, MutableSet<Long>>()
        private val lock = Any()

        /**
         * 注册一个 JS function 对象, 返回句柄。
         *
         * @param ctxPtr native JSContext 指针
         * @param jsObjectExpr JS 对象表达式 (如 "__jsHandler_123__", 指向全局作用域里的变量)
         * @param dangerousApi 是否旁路安全名单
         */
        fun register(ctxPtr: Long, jsObjectExpr: String, dangerousApi: Boolean): Long {
            val handle = handleCounter.getAndIncrement()
            val h = JsFunctionHandle(ctxPtr, jsObjectExpr, dangerousApi)
            synchronized(lock) {
                handleMap.put(handle, h)
            }
            trackHandle(handle)
            return handle
        }

        /**
         * 把 handle 记录到当前线程的 scope, close 时批量释放。
         * 调用方需在 [QuickJsEngine.eval] / [QuickJsEngine.injectBindings] 期间调用,
         * 这些路径已设置 [QuickJsContext.threadLocalContext]。
         */
        private fun trackHandle(handle: Long) {
            val scopeId = QuickJsContext.threadLocalContext.get()?.scopeId ?: return
            synchronized(lock) {
                var set = scopeHandles.get(scopeId)
                if (set == null) {
                    set = mutableSetOf()
                    scopeHandles.put(scopeId, set)
                }
                set.add(handle)
            }
        }

        fun get(handle: Long): JsFunctionHandle? {
            synchronized(lock) {
                return handleMap.get(handle)
            }
        }

        fun release(handle: Long) {
            synchronized(lock) {
                handleMap.remove(handle)
            }
        }

        /**
         * 释放某 scope 注册的全部 JsFunctionHandle 句柄, 由 [QuickJsContext.close] 调用。
         *
         * 避免全局 clearAll 影响其他活跃 scope。
         */
        fun releaseScope(scopeId: Long) {
            synchronized(lock) {
                val handles = scopeHandles.get(scopeId) ?: return
                handles.forEach { handleMap.remove(it) }
                scopeHandles.remove(scopeId)
            }
        }

        fun clearAll() {
            synchronized(lock) {
                handleMap.clear()
                scopeHandles.clear()
            }
        }
    }
}

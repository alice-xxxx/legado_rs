package io.legado.app.model.script.quickjs

import com.script.quickjs.CompiledScript
import com.script.quickjs.NativeObject
import com.script.quickjs.QuickJsContext
import com.script.quickjs.QuickJsEngine
import io.legado.app.model.script.*
import kotlin.coroutines.CoroutineContext

object QuickJsJsEngine : JsEngine {
    override val type: JsEngineType = JsEngineType.QUICKJS

    override fun eval(js: String, bindingsConfig: JsBindings.() -> Unit): Any? =
        QuickJsEngine.eval(js, JsBindings().apply(bindingsConfig))

    override fun eval(js: String, bindings: JsBindings): Any? = QuickJsEngine.eval(js, bindings)

    override fun getRuntimeScope(bindings: JsBindings): JsScope = QuickJsEngine.getRuntimeScope(bindings)
    override fun createStandaloneScope(): JsScope = QuickJsEngine.createQuickJsForActivity()
    override fun compile(script: String): JsCompiledScript = QuickJsEngine.compile(script)
    override fun compile(script: String, scope: JsScope): JsCompiledScript = QuickJsEngine.compile(script, scope.context())
    override fun wrapJsForEval(jsStr: String): String = QuickJsEngine.wrapJsForEval(jsStr)
    override fun compileForSubScope(jsStr: String): JsCompiledScript = QuickJsEngine.compileForSubScope(jsStr)

    override fun evalInSubScope(
        compiled: JsCompiledScript,
        scope: JsScope,
        bindings: JsBindings,
        coroutineContext: CoroutineContext?
    ): Any? = QuickJsEngine.evalInSubScope(
        compiled as? CompiledScript ?: error("QuickJS requires CompiledScript"),
        scope.context(), bindings, coroutineContext
    )

    override fun eval(js: String, scope: JsScope, coroutineContext: CoroutineContext?): Any? =
        QuickJsEngine.eval(js, scope.context(), coroutineContext)

    override fun eval(js: String, scope: JsScope, bindings: JsBindings, coroutineContext: CoroutineContext?): Any? =
        QuickJsEngine.eval(js, scope.context(), bindings, coroutineContext)

    override fun evalBytecode(bytecode: ByteArray?, scope: JsScope, coroutineContext: CoroutineContext?): Any? =
        if (bytecode == null) null else QuickJsEngine.evalBytecode(bytecode, scope.context(), coroutineContext)

    override fun injectBindings(scope: JsScope, bindings: JsBindings): List<String> =
        QuickJsEngine.injectBindings(scope.context(), bindings)

    override fun cleanupBindings(scope: JsScope, keys: List<String>) = QuickJsEngine.cleanupBindings(scope.context(), keys)
    override fun isJsObject(obj: Any?): Boolean = obj is NativeObject
    override fun asJsObject(obj: Any?): JsObject? = obj as? NativeObject
    override fun isJsException(t: Throwable): Boolean = t is com.script.quickjs.ScriptException
    override fun currentContext(): JsScope? = QuickJsContext.threadLocalContext.get()
    override fun currentContextNonNull(): JsScope = currentContext() ?: error("No QuickJsContext on current thread")

    override fun <T> runScriptWithContext(context: CoroutineContext, block: () -> T): T =
        com.script.quickjs.runScriptWithContext(context, block)

    private fun JsScope.context(): QuickJsContext = this as? QuickJsContext ?: error("QuickJS requires QuickJsContext")
}

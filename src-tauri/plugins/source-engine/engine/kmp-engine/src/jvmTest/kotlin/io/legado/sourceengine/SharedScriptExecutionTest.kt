package io.legado.sourceengine

import com.script.quickjs.JsFunction
import com.script.quickjs.JsFunctionHandle
import com.script.quickjs.JsStringUtils
import com.script.quickjs.QuickJsContext
import com.script.quickjs.ScriptBindings
import io.legado.app.model.script.*
import io.legado.sourceengine.host.HostRuntime
import kotlinx.coroutines.CoroutineName
import kotlinx.coroutines.Dispatchers
import kotlin.coroutines.ContinuationInterceptor
import kotlin.test.*

class SharedScriptExecutionTest {
    init { HostRuntime.install { error("Unexpected image operation") } }

    class Echo(private val text: String) {
        fun call(): Any? = JsFunction(QuickJsContext.threadLocalContext.get()!!, "function(x){return x;}").call(text)
        fun handleCall(): Any? {
            val scope = QuickJsContext.threadLocalContext.get()!!
            val handle = JsFunctionHandle.register(scope.ctxPtr, "function(x){return x;}", false)
            return JsFunctionHandle.get(handle)!!.invokeJsMethod("apply", arrayOf(text))
        }
    }

    class ContextProbe {
        fun call(): String {
            val scope = jsContext
            val original = scope.coroutineContext
            assertFails {
                runScriptWithContext(CoroutineName("nested") + Dispatchers.Default) {
                    assertEquals("nested", scope.coroutineContext!![CoroutineName]!!.name)
                    assertNull(scope.coroutineContext!![ContinuationInterceptor])
                    error("callback failure")
                }
            }
            assertSame(original, scope.coroutineContext)
            return "restored"
        }
    }

    @Test fun libraryEscapingPreservesScriptLiteralsAndCallbackArguments() {
        val engine = JsEngines.current
        val texts = listOf("", "中文😀\u2028\u2029", "\\\"/\n\r\t\b\u000C", (0..31).map(Int::toChar).joinToString(""))
        for (text in texts) {
            assertEquals(text, engine.eval(JsStringUtils.escape(text)))
            assertEquals(text, engine.eval("echo.call()", JsBindings().apply { this["echo"] = Echo(text) }))
            assertEquals(text, engine.eval("echo.handleCall()", JsBindings().apply { this["echo"] = Echo(text) }))
        }
    }

    @Test fun errorsAndRecursionLimitsRestoreTheOriginalExecutionState() {
        val engine = JsEngines.current
        engine.createStandaloneScope().use { scope ->
            val original = CoroutineName("caller")
            scope.coroutineContext = original
            scope.allowScriptRun = false
            scope.recursiveCount = 2
            assertFails { engine.eval("throw new Error('fixture')", scope, CoroutineName("evaluation")) }
            assertSame(original, scope.coroutineContext)
            assertFalse(scope.allowScriptRun)
            assertEquals(2, scope.recursiveCount)
            assertNull(engine.currentContext())
            scope.recursiveCount = 9
            assertFails { engine.eval("1", scope, null) }
            assertEquals(9, scope.recursiveCount)
            assertSame(original, scope.coroutineContext)
            assertFalse(scope.allowScriptRun)
            assertNull(engine.currentContext())
            scope.recursiveCount = 0
            assertEquals("42", engine.eval("42", scope, null).toString())
        }
    }

    @Test fun nestedCallbacksRestoreTheCallerCoroutineContext() {
        val engine = JsEngines.current
        engine.getRuntimeScope(JsBindings().apply { this["probe"] = ContextProbe() }).use { scope ->
            assertEquals("restored", engine.eval("probe.call()", scope, CoroutineName("caller")))
            assertNull(scope.coroutineContext)
        }
    }

    @Test fun bindingsAndCompiledValuesUseTheCommonContractsDirectly() {
        assertTrue(ScriptBindings().isEmpty())
        val bindings = JsBindings().apply { this["value"] = 42; dangerousApi = true }
        assertTrue(bindings.containsKey("platform"))
        assertTrue(bindings.containsKey("image"))
        val engine = JsEngines.current
        engine.getRuntimeScope(bindings).use { scope ->
            assertTrue((scope as QuickJsContext).dangerousApi)
            val compiled = engine.compile("({value:value})", scope)
            val value = compiled.eval(scope, null)
            assertTrue(engine.isJsObject(value))
            assertSame(value, engine.asJsObject(value))
            assertEquals("42", engine.asJsObject(value)!!["value"].toString())
        }
    }
}

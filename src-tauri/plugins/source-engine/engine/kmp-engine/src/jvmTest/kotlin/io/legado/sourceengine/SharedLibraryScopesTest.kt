package io.legado.sourceengine

import io.legado.app.data.entities.BookSource
import io.legado.app.model.script.*
import io.legado.sourceengine.host.HostRuntime
import kotlin.test.*

class SharedLibraryScopesTest {
    init { HostRuntime.install { error("Image operation outside fixture") } }

    private fun scope(code: String, url: String = "https://scope.invalid/"): JsScope =
        SharedLibraryScopes.getScope(code, false, null, JsBindings().apply {
            this["source"] = BookSource(bookSourceUrl = url)
        })!!

    @AfterTest fun clear() { SharedLibraryScopes.clearAll() }

    @Test fun versionsAndSourceBindingsHaveIndependentScopes() {
        val code = "var initialSource=source.getKey();var counter=0;function next(){return ++counter;}"
        val first = scope(code)
        val second = scope(code, "https://other.invalid/")
        val engine = JsEngines.get()
        assertSame(first, scope(code))
        assertEquals("1", engine.eval("next()", first, null).toString())
        assertEquals("1", engine.eval("next()", second, null).toString())
        assertEquals("https://other.invalid/", engine.eval("initialSource", second, null))
        SharedLibraryScopes.remove(code)
        val replacement = scope(code)
        assertNotSame(first, replacement)
        assertEquals("1", engine.eval("next()", replacement, null).toString())
    }

    @Test fun threadsNeverShareNativeContexts() {
        val code = "var value=1;"
        val first = scope(code)
        var other: JsScope? = null
        var failure: Throwable? = null
        Thread {
            try { other = scope(code); assertEquals("1", JsEngines.get().eval("value", other!!, null).toString()) }
            catch (error: Throwable) { failure = error }
            finally { SharedLibraryScopes.clearAll() }
        }.apply { start(); join() }
        failure?.let { throw it }
        assertNotSame(first, other)
    }

    class Eviction {
        fun evict() { SharedLibraryScopes.clearAll() }
    }

    @Test fun clearingDuringAJsCallbackDefersClosingTheActiveContext() {
        val owner = scope("var value=42;")
        assertEquals("42", JsEngines.get().eval("eviction.evict();value;", owner, JsBindings().apply {
            this["eviction"] = Eviction()
        }, null).toString())
        SharedLibraryScopes.clearAll()
        val context = (owner as io.legado.app.model.script.quickjs.QuickJsJsScope).ctx
        assertNull(com.script.quickjs.QuickJsContext.findByCtxPtr(context.ctxPtr))
        assertEquals("7", JsEngines.get().eval("7").toString())
    }

    @Test fun failedInitializationDoesNotPoisonTheCache() {
        assertFails { scope("throw new Error('library fixture');") }
        assertEquals("5", JsEngines.get().eval("value", scope("var value=5;"), null).toString())
    }
}

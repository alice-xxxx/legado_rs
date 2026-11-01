package io.legado.sourceengine

import com.script.jsdispatch.*
import com.script.quickjs.JsStringUtils
import io.legado.app.model.script.JsBindings
import io.legado.app.model.script.JsEngines
import io.legado.sourceengine.host.HostRuntime
import java.lang.reflect.InvocationTargetException
import kotlin.test.*

class SharedArgumentCoercionTest {
    init { HostRuntime.install { error("Unexpected image operation") } }

    @Test fun nullDefaultsAndInvalidPrimitiveArgumentsKeepTheirExceptionTypes() {
        assertEquals(0, JsCoerce.toInt(null))
        assertNull(JsCoerce.toIntOrNull(null))
        assertEquals(2.toByte(), JsCoerce.toByte(258.1))
        assertEquals(Int.MAX_VALUE, JsCoerce.toInt(Double.POSITIVE_INFINITY))
        assertEquals(0, JsCoerce.toInt(Double.NaN))
        assertFailsWith<ClassCastException> { JsCoerce.toInt("1") }
        assertFailsWith<IllegalArgumentException> { JsCoerce.toBooleanArg(null) }
        assertFailsWith<ClassCastException> { JsCoerce.toBooleanArg("true") }
        assertFailsWith<IllegalArgumentException> { JsCoerce.toBooleanOrNull("true") }
        assertEquals(' ', JsCoerce.toCharArg(""))
        assertEquals('a', JsCoerce.toCharArg("abc"))
        assertFailsWith<IllegalArgumentException> { JsCoerce.toCharOrNull("a") }
    }

    @Test fun arrayGuardsStillSeparateCleanArgumentsFromReflectionFallbacks() {
        val dirty = listOf(1, mapOf("handle" to 2))
        assertTrue(JsCoerce.isArrayLike(dirty))
        assertFalse(JsCoerce.isByteArrayStrict(dirty))
        assertFalse(JsCoerce.isStringArrayStrict(dirty))
        assertContentEquals(byteArrayOf(0, -1, -128), JsCoerce.toByteArrayArg(listOf(256, 255, 128)))
        assertContentEquals(arrayOf(null, "4", "false"), JsCoerce.toStringArrayArg(listOf(null, 4, false)))
        assertTrue(JsCoerce.isArrayLike(booleanArrayOf(true)))
        assertFalse(JsCoerce.isByteArrayStrict(booleanArrayOf(true)))
        assertNull(JsCoerce.toByteArrayArg(null))
        assertFailsWith<IllegalArgumentException> { JsCoerce.toByteArrayArg("bytes") }
    }

    @Test fun overloadScoresKeepBoxedAndRuntimeArrayTypesDistinct() {
        assertEquals(0, JsCoerce.scoreIntPrim(1))
        assertEquals(2, JsCoerce.scoreIntPrim(1.0))
        assertEquals(4, JsCoerce.scoreIntPrim(1.5))
        assertEquals(3, JsCoerce.scoreDoublePrim(1))
        assertEquals(0, JsCoerce.scoreDoublePrim(1.5))
        assertEquals(0, JsCoerce.scoreStringArray(arrayOf("a")))
        assertEquals(3, JsCoerce.scoreStringArray(arrayOf<Any>("a")))
        assertEquals(0, JsCoerce.scoreRef(null, List::class))
        assertEquals(1, JsCoerce.scoreRef(arrayListOf(1), List::class))
    }

    class Target { fun header(value: Map<String, String>): String = value.getValue("Referer") }

    class Overloaded {
        fun choose(value: Int): String = "int"
        fun choose(value: Double): String = "double"
        fun choose(value: Any?): String = "object"
    }

    @Test fun reflectiveOverloadsKeepTheirPreviousNumericPreference() {
        val target = Overloaded()
        fun choose(value: Any?): Any? = com.script.quickjs.JavaObjectBridge.callInstanceMethodRaw(target, "choose", arrayOf(value), false)
        assertEquals("int", choose(1))
        assertEquals("int", choose(2.toShort()))
        assertEquals("int", choose(Long.MIN_VALUE))
        assertEquals("double", choose(1.0))
        assertEquals("double", choose(1.5f))
        assertEquals("double", choose(java.math.BigDecimal("1")))
        assertEquals("object", choose("1"))
        assertEquals("object", choose(null))
    }

    @Test fun generatedAndReflectiveMapArgumentsUseTheExistingJsonLibraryBehavior() {
        val input = "{Referer:'正文',empty:null,n:1,b:true,list:[1,2],obj:{k:2}}"
        assertEquals(mapOf("Referer" to "正文", "empty" to "null", "n" to "1", "b" to "true",
            "list" to "[1,2]", "obj" to "{\"k\":2}"), JsCoerce.toMapArg(input))
        val map = mapOf("a" to "b")
        assertSame(map, JsCoerce.toMapArg(map))
        assertFailsWith<IllegalArgumentException> { JsCoerce.toMapArg("[1,2]") }
        val script = "target.header(" + JsStringUtils.escape(input) + ")"
        assertEquals("正文", JsEngines.current.eval(script, JsBindings().apply { this["target"] = Target() }))
    }

    @Test fun targetExceptionsRemainWrappedSeparatelyFromCoercionFailures() {
        val failure = IllegalStateException("fixture")
        assertSame(failure, assertFailsWith<InvocationTargetException> { JsCoerce.invokeTarget { throw failure } }.targetException)
        assertFailsWith<ClassCastException> { JsCoerce.toInt("bad") }
        assertEquals(42, JsCoerce.invokeTarget { JsCoerce.toInt(42.0) })
    }
}

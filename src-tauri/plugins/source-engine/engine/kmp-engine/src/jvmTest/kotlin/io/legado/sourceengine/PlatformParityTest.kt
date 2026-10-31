package io.legado.sourceengine

import io.legado.app.exception.RegexTimeoutException
import io.legado.app.model.script.JsBindings
import io.legado.app.model.script.JsEngines
import io.legado.app.utils.DeadlineRegexReplacer
import io.legado.sourceengine.host.HostRuntime
import kotlin.test.*

class PlatformParityTest {
    init { HostRuntime.install { error("Image operation outside fixture") } }

    @Test fun regexReplacementKeepsGroupsEscapesAndEmptyMatches() {
        val source = "a1 a2"
        val regex = Regex("(?<letter>[a-z])(\\d)")
        for (replacement in listOf("\${letter}:$2", "\\$1", "$0", "$12")) {
            val expected = regex.toPattern().matcher(source).replaceAll(replacement)
            assertEquals(expected, DeadlineRegexReplacer.replace(source, regex, replacement, 1000))
        }
        assertEquals("-a-b-", DeadlineRegexReplacer.replace("ab", Regex(""), "-", 1000))
        assertEquals("--", DeadlineRegexReplacer.replace("ab", Regex(".*"), "-", 1000))
        assertEquals("bb", DeadlineRegexReplacer.replace("a", Regex("a|$"), "b", 1000))
        assertEquals("unchanged", DeadlineRegexReplacer.replace("unchanged", Regex("x"), "$1", 1000))
    }

    @Test fun regexDeadlinesStopMatchingAndDoNotPoisonTheNextCall() {
        assertFailsWith<RegexTimeoutException> { DeadlineRegexReplacer.replace("a", Regex("a"), "b", 0) }
        assertFailsWith<RegexTimeoutException> {
            DeadlineRegexReplacer.replace("a".repeat(10000) + "!", Regex("(a+)+$"), "", 20)
        }
        assertEquals("b", DeadlineRegexReplacer.replace("a", Regex("a"), "b", Long.MAX_VALUE))
    }

    @Test fun jsReplacementIsLiteralAndCompileFailureDoesNotBreakLaterCalls() {
        assertEquals("$1$1", DeadlineRegexReplacer.replace("aa", Regex("a"), "@js:'$1'", 5000))
        assertEquals("AA", DeadlineRegexReplacer.replace("aa", Regex("a"), "@js:result.toUpperCase()", 5000))
        assertEquals("aa", DeadlineRegexReplacer.replace("aa", Regex("x"), "@js:function(", 5000))
        assertFails { DeadlineRegexReplacer.replace("a", Regex("a"), "@js:function(", 5000) }
        assertEquals("b", DeadlineRegexReplacer.replace("a", Regex("a"), "@js:'b'", 5000))
    }

    @Test fun utf16ValuesAndKeysRoundTripAcrossTheJvmBridge() {
        val text = "我的\u0000😀\uD800尾"
        val engine = JsEngines.get()
        val scope = engine.getRuntimeScope(JsBindings().apply {
            this["text"] = text
            this["values"] = mapOf(text to text)
        })
        try {
            assertEquals(text, engine.eval("text", scope, null))
            assertEquals(text, engine.eval("values[text]", scope, null))
            val result = engine.eval("({[text]:text})", scope, null) as Map<*, *>
            assertEquals(text, result[text])
        } finally {
            scope.close()
        }
    }

    @Test fun jvmInterfacesAcceptFunctionsAndObjectImplementations() {
        assertEquals("6:called", JsEngines.get().eval("""
            var values=new java.util.ArrayList();values.add(3);values.add(1);values.add(2);
            values.sort(new java.util.Comparator((a,b)=>a-b));
            var sum=0;values.forEach(new java.util.function.Consumer(x=>{sum+=x;}));
            var state='';new java.lang.Runnable({run:function(){state='called';}}).run();
            sum+':'+state;
        """).toString())
    }
}

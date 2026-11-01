package io.legado.sourceengine

import com.script.quickjs.ClassNameMatcher
import kotlin.test.*

class SharedClassMatcherTest {
    @Test fun librarySearchAndSharedCacheKeepClassAndPackageBoundaries() {
        val matcher = ClassNameMatcher(listOf("sun", "java.io.File", "java.lang.reflect", "java.io.File"))
        val cases = mapOf("sun" to true, "sun.net.Connection" to true, "sunny" to false,
            "java.io.File" to true, "java.io.File.foo" to true, "java.io.FileInputStream" to false,
            "java.io.File\$Nested" to false, "java.lang.reflect.Method" to true,
            "java.lang.reflective" to false, "java.lang.String" to false, "" to false)
        repeat(3) {
            for ((name, expected) in cases) assertEquals(expected, matcher.match(name), name)
            repeat(100) { assertFalse(matcher.match("other.Type" + it)) }
        }
        assertFalse(ClassNameMatcher(emptyList()).match("sun"))
    }
}

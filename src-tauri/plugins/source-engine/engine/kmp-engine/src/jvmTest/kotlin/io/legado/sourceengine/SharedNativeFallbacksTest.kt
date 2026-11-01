package io.legado.sourceengine

import io.legado.app.utils.concurrent.newSynchronizedMap
import io.legado.app.utils.formatWordCountPlain
import io.legado.app.utils.cnCompareFallback
import java.util.concurrent.atomic.AtomicReference
import kotlin.concurrent.thread
import kotlin.test.*

class SharedNativeFallbacksTest {
    @Test fun chineseFallbackKeepsItsExistingPinyinOrder() {
        val sorted = listOf("中", "西", "上", "南", "北").sortedWith { a, b -> a.cnCompareFallback(b) }
        assertEquals(listOf("北", "南", "上", "西", "中"), sorted)
        assertEquals(0, "北京".cnCompareFallback("北京"))
        assertTrue("北京".cnCompareFallback("北京站") < 0)
        assertTrue("伊".cnCompareFallback("衣") < 0)
    }

    @Test fun chineseFallbackKeepsCodePointRulesOutsideThePinyinTable() {
        assertTrue("A".cnCompareFallback("a") < 0)
        assertTrue("2".cnCompareFallback("10") > 0)
        assertTrue("鿿".cnCompareFallback("一") > 0)
        assertTrue("😀".cnCompareFallback("中") > 0)
        assertTrue("".cnCompareFallback("a") < 0)
    }

    @Test fun fixedWordCountsKeepTheirOriginalRoundingAndSpecialValues() {
        for ((value, expected) in listOf(0.0 to "0", -0.0 to "0", 1.0 to "1", 1.25 to "1.3",
            -1.25 to "-1.2", 0.05 to "0.1", -0.05 to "-0", Double.NaN to "NaN",
            Double.POSITIVE_INFINITY to "Infinity", Double.NEGATIVE_INFINITY to "-Infinity")) {
            assertEquals(expected, formatWordCountPlain(value))
        }
    }

    @Test fun mapIterationSnapshotsKeepTheirOriginalDetachedBehavior() {
        val map = newSynchronizedMap<String, Int>().apply { put("a", 1); put("b", 2) }
        val entries = map.entries
        val keys = map.keys
        val values = map.values
        map.clear()
        assertEquals(setOf("a", "b"), keys)
        assertEquals(listOf(1, 2), values.toList())
        val entry = entries.first { it.key == "a" }
        assertEquals(1, entry.setValue(3))
        assertEquals(3, entry.value)
        keys.clear()
        values.clear()
        assertTrue(map.isEmpty())
        assertEquals(setOf("a", "b"), entries.map { it.key }.toSet())
    }

    @Test fun concurrentWritesAndSnapshotIterationDoNotLoseEntriesOrThrow() {
        val map = newSynchronizedMap<Int, Int>()
        val failure = AtomicReference<Throwable?>()
        val workers = (0..3).map { worker -> thread {
            try {
                for (index in 0 until 1000) {
                    val key = worker * 1000 + index
                    map[key] = key
                    if (index % 25 == 0) map.entries.forEach { assertEquals(it.key, it.value) }
                }
            } catch (error: Throwable) { failure.compareAndSet(null, error) }
        } }
        workers.forEach { it.join() }
        failure.get()?.let { throw it }
        assertEquals(4000, map.size)
        assertTrue((0 until 4000).all { map[it] == it })
    }
}

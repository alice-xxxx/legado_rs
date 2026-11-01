package io.legado.sourceengine

import io.legado.app.model.analyzeRule.AnalyzeUrlCore
import io.legado.app.utils.formatIntegralDouble
import io.legado.sourceengine.host.HostRuntime
import java.util.Locale
import kotlin.random.Random
import kotlin.test.*

class SharedRuleNumbersTest {
    @Test fun integralNumbersKeepPlainDecimalWithoutIntOrLongSaturation() {
        val random = Random(71)
        val values = listOf(0.0, -0.0, 2147483648.0, -2147483649.0, Long.MAX_VALUE.toDouble(),
            1e20, -1e20, Double.MAX_VALUE, -Double.MAX_VALUE) +
            List(3000) { Double.fromBits(random.nextLong()) }.filter { it.isFinite() && it % 1.0 == 0.0 }
        for (value in values) {
            assertEquals(String.format(Locale.ROOT, "%.0f", value), formatIntegralDouble(value), value.toString())
        }
    }

    @Test fun sourceUrlInterpolationUsesTheSharedIntegralConversion() {
        HostRuntime.install { error("Unexpected image operation") }
        assertEquals("https://fixture.invalid/2147483648", AnalyzeUrlCore("https://fixture.invalid/{{2147483648.0}}").url)
        assertEquals("https://fixture.invalid/100000000000000000000", AnalyzeUrlCore("https://fixture.invalid/{{1e20}}").url)
        assertEquals("https://fixture.invalid/-100000000000000000000", AnalyzeUrlCore("https://fixture.invalid/{{-1e20}}").url)
    }
}

package io.legado.sourceengine

import io.legado.app.help.formatSimpleUtcTime
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.SimpleTimeZone
import kotlin.test.*

class SharedDatePatternTest {
    @Test fun supportedNumericPatternsMatchTheDateLibrary() {
        for (time in listOf(-1L, 0L, 1L, 951782400009L, 1_700_000_000_123L)) {
            for (hours in listOf(-8, 0, 8)) {
                for (pattern in listOf("yyyy-MM-dd HH:mm:ss.SSS", "yy M d H:m:s.S", "yyyyMMddHHmmss")) {
                    val jdk = SimpleDateFormat(pattern, Locale.ROOT).apply { timeZone = SimpleTimeZone(hours * 3_600_000, "UTC") }
                    assertEquals(jdk.format(Date(time)), formatSimpleUtcTime(time, pattern, hours))
                }
            }
        }
    }

    @Test fun nativePatternWidthsAndLiteralRulesRemainUnchanged() {
        assertEquals("70 70 70 1970 1970 1 01 01 01 '1970' 9 9 009 009",
            formatSimpleUtcTime(9, "y yy yyy yyyy yyyyy M MM MMM MMMM 'yyyy' S SS SSS SSSS", 0))
        assertEquals("1969-12-31 23:59:59.999", formatSimpleUtcTime(-1, "yyyy-MM-dd HH:mm:ss.SSS", 0))
        assertEquals("1970-01-01 08:00", formatSimpleUtcTime(0, "yyyy-MM-dd HH:mm", 8))
    }
}

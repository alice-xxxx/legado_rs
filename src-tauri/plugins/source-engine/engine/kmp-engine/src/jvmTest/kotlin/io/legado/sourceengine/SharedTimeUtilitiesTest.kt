package io.legado.sourceengine

import io.legado.app.constant.ThreadSafeDateFormat
import io.legado.app.utils.*
import java.time.LocalDate
import java.time.format.DateTimeFormatter
import java.util.Locale
import java.util.TimeZone
import kotlin.test.*
import kotlin.time.Instant

class SharedTimeUtilitiesTest {
    private fun inZone(id: String, block: () -> Unit) {
        val previous = TimeZone.getDefault()
        TimeZone.setDefault(TimeZone.getTimeZone(id))
        try { block() } finally { TimeZone.setDefault(previous) }
    }

    @Test fun datesAndMidnightRespectHistoricalTimeZonesAndLeapDays() {
        for (id in listOf("UTC", "Asia/Shanghai", "America/New_York", "America/Sao_Paulo")) inZone(id) {
            for (text in listOf("1970-01-01", "2000-02-29", "2024-03-10", "2024-11-03", "2018-11-04")) {
                val date = LocalDate.parse(text)
                val midnight = date.atStartOfDay(java.time.ZoneId.of(id)).toEpochSecond()
                val key = date.year * 10000 + date.monthValue * 100 + date.dayOfMonth
                assertEquals(midnight, midnightSecFromDayKey(key), "$id $text")
                val before = date.minusDays(1)
                assertEquals(before.year * 10000 + before.monthValue * 100 + before.dayOfMonth, prevDayKey(key))
                assertEquals(Triple(date.year, date.monthValue, date.dayOfMonth), yearMonthDayFromMillis(midnight * 1000))
            }
        }
        assertEquals(20250201, prevDayKey(20241333))
    }

    @Test fun libraryAndSharedNumericFormatsMatchJdk() {
        val patterns = listOf("HH:mm", "yyyy/MM/dd HH:mm", "yy-MM-dd-HH-mm-ss")
        for (id in listOf("UTC", "Asia/Shanghai", "America/New_York")) inZone(id) {
            for (text in listOf("1899-12-31T23:59:59Z", "1969-12-31T23:59:59Z", "2024-03-10T06:59:59Z", "2024-03-10T07:00:00Z", "2024-11-03T06:30:00Z", "2100-01-01T00:00:00Z")) {
                val instant = Instant.parse(text)
                for (pattern in patterns) {
                    val expected = DateTimeFormatter.ofPattern(pattern, Locale.ROOT)
                        .withZone(java.time.ZoneId.of(id)).format(java.time.Instant.ofEpochMilli(instant.toEpochMilliseconds()))
                    assertEquals(expected, ThreadSafeDateFormat(pattern).format(instant.toEpochMilliseconds()))
                }
                val time = DateTimeFormatter.ofPattern("HH:mm", Locale.ROOT).withZone(java.time.ZoneId.of(id))
                    .format(java.time.Instant.ofEpochMilli(instant.toEpochMilliseconds()))
                assertEquals(time, formatTimeOfDay(instant.toEpochMilliseconds()))
            }
        }
    }

    @Test fun wallClockAndMonotonicClockKeepTheirContracts() {
        val before = System.currentTimeMillis()
        val wall = systemCurrentTimeMillis()
        assertTrue(wall in before..System.currentTimeMillis())
        val first = systemNanoTime()
        assertTrue(systemNanoTime() >= first)
    }
}

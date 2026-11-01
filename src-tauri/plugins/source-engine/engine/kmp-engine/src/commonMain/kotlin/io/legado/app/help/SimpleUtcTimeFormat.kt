package io.legado.app.help

import kotlinx.datetime.TimeZone
import kotlinx.datetime.toLocalDateTime
import kotlin.time.Instant

/** Numeric date-pattern fallback; retain the existing Native widths and literal handling. */
internal fun formatSimpleUtcTime(time: Long, pattern: String, shiftHours: Int): String {
    val date = Instant.fromEpochMilliseconds(time + shiftHours * 3_600_000L).toLocalDateTime(TimeZone.UTC)
    return buildString(pattern.length + 8) {
        var start = 0
        while (start < pattern.length) {
            val token = pattern[start]
            var end = start + 1
            while (end < pattern.length && pattern[end] == token) end++
            val count = end - start
            val value = when (token) {
                'y' -> if (count >= 4) date.year else date.year % 100
                'M' -> date.month.ordinal + 1
                'd' -> date.day
                'H' -> date.hour
                'm' -> date.minute
                's' -> date.second
                'S' -> date.nanosecond / 1_000_000
                else -> null
            }
            val width = when (token) {
                'y' -> if (count >= 4) 4 else 2
                'S' -> if (count >= 3) 3 else 1
                else -> if (count >= 2) 2 else 1
            }
            if (value == null) append(pattern, start, end) else append(value.toString().padStart(width, '0'))
            start = end
        }
    }
}

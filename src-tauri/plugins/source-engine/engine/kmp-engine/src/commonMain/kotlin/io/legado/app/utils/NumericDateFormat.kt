package io.legado.app.utils

import kotlinx.datetime.LocalDateTime
import kotlinx.datetime.TimeZone
import kotlinx.datetime.format.FormatStringsInDatetimeFormats
import kotlinx.datetime.format.byUnicodePattern
import kotlinx.datetime.toLocalDateTime
import kotlin.time.Instant

internal fun localDateTime(epochMillis: Long): LocalDateTime =
    Instant.fromEpochMilliseconds(epochMillis).toLocalDateTime(TimeZone.currentSystemDefault())

/** Numeric formats used by AppConst, with Java's fixed two-digit year prefix. */
@OptIn(FormatStringsInDatetimeFormats::class)
internal class NumericDateFormat(pattern: String) {
    private val shortYear = pattern.startsWith("yy") && !pattern.startsWith("yyy")
    private val format = LocalDateTime.Format { byUnicodePattern(if (shortYear) pattern.drop(2) else pattern) }

    fun format(millis: Long): String {
        val date = localDateTime(millis)
        val prefix = if (shortYear) date.year.mod(100).toString().padStart(2, '0') else ""
        return prefix + format.format(date)
    }
}

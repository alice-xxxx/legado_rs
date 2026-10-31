package io.legado.app.utils

import kotlinx.datetime.*
import kotlin.time.Clock
import kotlin.time.TimeSource

private val monotonicOrigin = TimeSource.Monotonic.markNow()
private val hourMinuteFormat = NumericDateFormat("HH:mm")

fun systemCurrentTimeMillis(): Long = Clock.System.now().toEpochMilliseconds()
fun systemNanoTime(): Long = monotonicOrigin.elapsedNow().inWholeNanoseconds

fun yearMonthDayFromMillis(epochMillis: Long): Triple<Int, Int, Int> =
    localDateTime(epochMillis).let { Triple(it.year, it.monthNumber, it.day) }

// Preserve Calendar's normalization of overflowed month/day fields.
private fun dateFromKey(key: Int): LocalDate = LocalDate(key / 10000, 1, 1)
    .plus(key / 100 % 100 - 1, DateTimeUnit.MONTH)
    .plus(key % 100 - 1, DateTimeUnit.DAY)

fun prevDayKey(dayKey: Int): Int = dateFromKey(dayKey).minus(1, DateTimeUnit.DAY)
    .let { it.year * 10000 + it.monthNumber * 100 + it.day }

fun midnightSecFromDayKey(dayKey: Int): Long =
    dateFromKey(dayKey).atStartOfDayIn(TimeZone.currentSystemDefault()).epochSeconds

fun formatTimeOfDay(epochMillis: Long): String = hourMinuteFormat.format(epochMillis)

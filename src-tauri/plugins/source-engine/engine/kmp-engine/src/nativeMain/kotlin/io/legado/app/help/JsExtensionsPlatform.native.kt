package io.legado.app.help

internal expect fun isMainThreadPlatform(): Boolean

internal actual object JsPlatformRuntime {
    actual fun formatTimeUtc(time: Long, format: String, shiftHours: Int): String {
        val shiftedMillis = time + shiftHours * 3_600_000L
        val (y, mo, d) = io.legado.app.utils.yearMonthDayFromMillis(shiftedMillis)
        // 计算时分秒 (UTC 当天)
        val dayMillis = shiftedMillis.mod(86_400_000L).toInt()
        val totalSeconds = dayMillis / 1000
        val hour = totalSeconds / 3600
        val minute = (totalSeconds % 3600) / 60
        val second = totalSeconds % 60
        val millis = dayMillis % 1000
        return formatSimpleDate(format, y, mo, d, hour, minute, second, millis)
    }
    actual fun isMainThread(): Boolean = isMainThreadPlatform()
    actual fun unsafeSslContext(): Any? = null
}

private fun formatSimpleDate(
    pattern: String,
    year: Int,
    month: Int,
    day: Int,
    hour: Int,
    minute: Int,
    second: Int,
    millis: Int,
): String {
    val sb = StringBuilder(pattern.length + 8)
    var i = 0
    while (i < pattern.length) {
        val c = pattern[i]
        when (c) {
            'y' -> {
                val n = countRepeat(pattern, i, 'y')
                if (n >= 4) sb.append(year.toString().padStart(4, '0'))
                else sb.append((year % 100).toString().padStart(2, '0'))
                i += n
            }
            'M' -> {
                val n = countRepeat(pattern, i, 'M')
                sb.append(if (n >= 2) month.toString().padStart(2, '0') else month.toString())
                i += n
            }
            'd' -> {
                val n = countRepeat(pattern, i, 'd')
                sb.append(if (n >= 2) day.toString().padStart(2, '0') else day.toString())
                i += n
            }
            'H' -> {
                val n = countRepeat(pattern, i, 'H')
                sb.append(if (n >= 2) hour.toString().padStart(2, '0') else hour.toString())
                i += n
            }
            'm' -> {
                val n = countRepeat(pattern, i, 'm')
                sb.append(if (n >= 2) minute.toString().padStart(2, '0') else minute.toString())
                i += n
            }
            's' -> {
                val n = countRepeat(pattern, i, 's')
                sb.append(if (n >= 2) second.toString().padStart(2, '0') else second.toString())
                i += n
            }
            'S' -> {
                val n = countRepeat(pattern, i, 'S')
                if (n >= 3) sb.append(millis.toString().padStart(3, '0'))
                else sb.append(millis.toString())
                i += n
            }
            else -> {
                sb.append(c)
                i++
            }
        }
    }
    return sb.toString()
}

private fun countRepeat(s: String, start: Int, c: Char): Int {
    var n = 0
    var i = start
    while (i < s.length && s[i] == c) {
        n++
        i++
    }
    return n
}

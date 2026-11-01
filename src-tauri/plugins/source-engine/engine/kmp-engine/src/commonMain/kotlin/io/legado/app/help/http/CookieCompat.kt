package io.legado.app.help.http

/** Cookie fallback for runtimes without OkHttp. Retains existing Native domain/date acceptance;
 * public-suffix and IDN processing remain the responsibility of the platform cookie library. */
internal fun <T : Any> parseCompatCookies(
    url: String, headers: Map<String, List<String>>, create: (String, String, Boolean) -> T
): List<T> {
    val urlHost = url.cookieHostName() ?: return emptyList()
    val result = mutableListOf<T>()
    for ((key, values) in headers) {
        if (key.equals("Set-Cookie", ignoreCase = true)) {
            for (header in values) {
                parseSetCookieHeader(urlHost, header, create)?.let { result.add(it) }
            }
        }
    }
    return result
}

private fun <T : Any> parseSetCookieHeader(urlHost: String, header: String, create: (String, String, Boolean) -> T): T? {
    val cookiePairEnd = delimiterOffset(header, ';', 0, header.length)
    val pairEqualsSign = delimiterOffset(header, '=', 0, cookiePairEnd)
    if (pairEqualsSign == cookiePairEnd) return null

    val name = header.substring(0, pairEqualsSign).trim()
    if (name.isEmpty() || name.hasControlOrNonAscii()) return null
    val value = header.substring(pairEqualsSign + 1, cookiePairEnd).trim()
    if (value.hasControlOrNonAscii()) return null

    var domain: String? = null
    var persistent = false
    var pos = cookiePairEnd + 1
    while (pos < header.length) {
        val attributePairEnd = delimiterOffset(header, ';', pos, header.length)
        val attributeEqualsSign = delimiterOffset(header, '=', pos, attributePairEnd)
        val attributeName = header.substring(pos, attributeEqualsSign).trim()
        val attributeValue =
            if (attributeEqualsSign < attributePairEnd) {
                header.substring(attributeEqualsSign + 1, attributePairEnd).trim()
            } else {
                ""
            }

        when {
            attributeName.equals("expires", ignoreCase = true) -> {
                if (validCookieDate(attributeValue)) {
                    persistent = true
                }
            }

            attributeName.equals("max-age", ignoreCase = true) -> {
                if (validMaxAge(attributeValue)) {
                    persistent = true
                }
            }

            attributeName.equals("domain", ignoreCase = true) -> {
                parseDomain(attributeValue)?.let { domain = it }
            }
        }

        pos = attributePairEnd + 1
    }

    if (domain != null && !domainMatch(urlHost, domain)) {
        return null
    }

    return create(name, value, persistent)
}

private fun delimiterOffset(s: String, c: Char, start: Int, end: Int): Int =
    s.indexOf(c, start).takeIf { it in start until end } ?: end

private fun String.hasControlOrNonAscii(): Boolean =
    any { it.code !in 0x20..0x7e }

private val MAX_AGE_PATTERN = Regex("-?\\d+")
private fun validMaxAge(value: String): Boolean = value.toLongOrNull() != null || MAX_AGE_PATTERN.matches(value)

private fun parseDomain(s: String): String? {
    if (s.endsWith(".")) return null
    val d = s.removePrefix(".").lowercase()
    if (d.isEmpty()) return null
    if (d.any { it !in 'a'..'z' && it !in '0'..'9' && it != '-' && it != '.' }) return null
    if (d.length !in 1..253) return null
    if (d.split('.').any { it.isEmpty() || it.length > 63 }) return null
    return d
}

private fun domainMatch(urlHost: String, domain: String): Boolean {
    if (urlHost == domain) return true
    if (urlHost.isIpAddress()) return false
    if (!urlHost.endsWith(domain)) return false
    val dotIndex = urlHost.length - domain.length - 1
    return dotIndex >= 0 && urlHost[dotIndex] == '.'
}

private fun String.isIpAddress(): Boolean {
    if (contains(':')) return true
    val parts = split('.')
    return parts.size == 4 && parts.all { it.isNotEmpty() && it.all(Char::isDigit) }
}

private val TIME_PATTERN = Regex("""(\d{1,2}):(\d{1,2}):(\d{1,2})""")
private val DAY_OF_MONTH_PATTERN = Regex("""(\d{1,2})[^\d]*""")
private val YEAR_PATTERN = Regex("""(\d{2,4})[^\d]*""")
private val MONTH_NAMES =
    listOf("jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec")

private fun validCookieDate(s: String): Boolean {
    var hour = -1
    var minute = -1
    var second = -1
    var dayOfMonth = -1
    var month = -1
    var year = -1

    val tokens = s.split(NON_DATE_CHAR_PATTERN).filter { it.isNotEmpty() }
    for (token in tokens) {
        when {
            hour == -1 && TIME_PATTERN.matches(token) -> {
                val m = TIME_PATTERN.matchEntire(token)!!
                hour = m.groupValues[1].toInt()
                minute = m.groupValues[2].toInt()
                second = m.groupValues[3].toInt()
            }

            dayOfMonth == -1 && DAY_OF_MONTH_PATTERN.matches(token) -> {
                dayOfMonth = DAY_OF_MONTH_PATTERN.matchEntire(token)!!.groupValues[1].toInt()
            }

            month == -1 && MONTH_NAMES.any { token.lowercase().startsWith(it) } -> {
                month = MONTH_NAMES.indexOfFirst { token.lowercase().startsWith(it) } + 1
            }

            year == -1 && YEAR_PATTERN.matches(token) -> {
                year = YEAR_PATTERN.matchEntire(token)!!.groupValues[1].toInt()
            }
        }
    }

    if (year in 70..99) year += 1900
    if (year in 0..69) year += 2000

    if (year < 1601 || month == -1 || dayOfMonth !in 1..31 ||
        hour !in 0..23 || minute !in 0..59 || second !in 0..59
    ) {
        return false
    }

    return true
}

private val NON_DATE_CHAR_PATTERN = Regex("""[^0-9A-Za-z:]+""")

private fun String.cookieHostName(): String? {
    val s = this
    val schemeEnd = s.indexOf("://")
    if (schemeEnd == -1) return null
    var hostPort = s.substring(schemeEnd + 3)
    val pathStart = hostPort.indexOf('/')
    if (pathStart != -1) hostPort = hostPort.substring(0, pathStart)
    val at = hostPort.lastIndexOf('@')
    if (at != -1) hostPort = hostPort.substring(at + 1)
    if (hostPort.startsWith("[")) {
        val close = hostPort.indexOf(']')
        return if (close != -1) hostPort.substring(1, close).lowercase() else null
    }
    val colon = hostPort.indexOf(':')
    return (if (colon != -1) hostPort.substring(0, colon) else hostPort).lowercase()
}

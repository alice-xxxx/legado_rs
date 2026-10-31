package io.legado.app.utils

/** Legacy ASCII escaping rules and IP validation shared by every target. */
internal object NetworkValueChecks {
    fun encodedQuery(value: String): Boolean = encoded(value, "!$&()*+,-./:;=?@[\\]^_`{|}~")
    fun encodedForm(value: String): Boolean = encoded(value, "*-._")

    private fun encoded(value: String, safe: String): Boolean {
        var index = 0
        while (index < value.length) {
            val char = value[index]
            if (char in 'a'..'z' || char in 'A'..'Z' || char in '0'..'9' || char in safe) index++
            else if (char == '%' && index + 2 < value.length &&
                value[index + 1] in "0123456789ABCDEFabcdef" && value[index + 2] in "0123456789ABCDEFabcdef") index += 3
            else return false
        }
        return true
    }

    private val ipv4Regex = Regex(
        "^(25[0-5]|2[0-4]\\d|[0-1]?\\d?\\d)\\.(25[0-5]|2[0-4]\\d|[0-1]?\\d?\\d)\\." +
            "(25[0-5]|2[0-4]\\d|[0-1]?\\d?\\d)\\.(25[0-5]|2[0-4]\\d|[0-1]?\\d?\\d)$"
    )
    private val ipv6Regex = Regex(
        "(([0-9a-fA-F]{1,4}:){7}[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,7}:|" +
            "([0-9a-fA-F]{1,4}:){1,6}:[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,5}(:[0-9a-fA-F]{1,4}){1,2}|" +
            "([0-9a-fA-F]{1,4}:){1,4}(:[0-9a-fA-F]{1,4}){1,3}|([0-9a-fA-F]{1,4}:){1,3}(:[0-9a-fA-F]{1,4}){1,4}|" +
            "([0-9a-fA-F]{1,4}:){1,2}(:[0-9a-fA-F]{1,4}){1,5}|[0-9a-fA-F]{1,4}:((:[0-9a-fA-F]{1,4}){1,6})|" +
            ":((:[0-9a-fA-F]{1,4}){1,7}|:)|fe80:(:[0-9a-fA-F]{0,4}){0,4}%[0-9a-zA-Z]+|" +
            "::(ffff(:0{1,4})?:)?((25[0-5]|(2[0-4]|1?[0-9])?[0-9])\\.){3}(25[0-5]|(2[0-4]|1?[0-9])?[0-9])|" +
            "([0-9a-fA-F]{1,4}:){1,4}:((25[0-5]|(2[0-4]|1?[0-9])?[0-9])\\.){3}(25[0-5]|(2[0-4]|1?[0-9])?[0-9]))"
    )

    fun isIPv4Address(input: String?): Boolean = !input.isNullOrEmpty() &&
        input[0] in '1'..'9' && input.count { it == '.' } == 3 && ipv4Regex.matches(input)

    fun isIPv6Address(input: String?): Boolean = input != null && input.contains(':') && ipv6Regex.matches(input)
    fun isIPAddress(input: String?): Boolean = isIPv4Address(input) || isIPv6Address(input)
}

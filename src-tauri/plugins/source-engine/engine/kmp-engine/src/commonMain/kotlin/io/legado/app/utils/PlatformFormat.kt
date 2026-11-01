package io.legado.app.utils

/** Plain decimal form for the integral Double results returned by source scripts. */
internal fun formatIntegralDouble(value: Double): String {
    val text = value.toString()
    val exponentAt = text.indexOf('E', ignoreCase = true)
    if (exponentAt < 0) return text.substringBefore('.')
    val mantissa = text.substring(0, exponentAt)
    val decimalAt = mantissa.indexOf('.').takeIf { it >= 0 } ?: mantissa.length
    val length = decimalAt + text.substring(exponentAt + 1).toInt()
    return mantissa.replace(".", "").padEnd(length, '0').take(length)
}

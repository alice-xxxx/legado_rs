package io.legado.app.utils

/** Keep table pronunciation, same-pinyin code-point ties and unknown-character fallback. */
internal fun String.cnCompareFallback(other: String): Int {
    val len = minOf(length, other.length)
    for (i in 0 until len) {
        val c1 = this[i]
        val c2 = other[i]
        if (c1 == c2) continue
        val p1 = PinyinTable.pinyin(c1)
        val p2 = PinyinTable.pinyin(c2)
        if (p1 != null && p2 != null) {
            val cmp = p1.compareTo(p2)
            if (cmp != 0) return cmp
            return c1.code.compareTo(c2.code)
        }
        return c1.code.compareTo(c2.code)
    }
    return length.compareTo(other.length)
}

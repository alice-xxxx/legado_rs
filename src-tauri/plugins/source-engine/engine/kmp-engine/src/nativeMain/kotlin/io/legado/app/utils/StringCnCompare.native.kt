package io.legado.app.utils

actual fun String.cnCompare(other: String): Int = cnCompareFallback(other)

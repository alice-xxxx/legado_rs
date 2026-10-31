package io.legado.app.utils

actual typealias URL = java.net.URL

actual fun URL.urlQuery(): String? = query

actual typealias File = java.io.File

actual fun Throwable.isSecurityException(): Boolean = this is SecurityException

actual fun String.platformIntern(): String = intern()

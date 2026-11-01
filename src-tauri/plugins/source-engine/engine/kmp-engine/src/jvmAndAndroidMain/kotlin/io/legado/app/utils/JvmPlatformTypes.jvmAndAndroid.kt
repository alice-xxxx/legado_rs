package io.legado.app.utils

actual typealias URL = java.net.URL

actual fun URL.urlQuery(): String? = query

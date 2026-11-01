package io.legado.app.help.http

actual class KmpCookie(
    actual val name: String,
    actual val value: String,
    actual val persistent: Boolean
)

actual fun parseResponseCookies(url: KmpHttpUrl, headers: KmpHeaders): List<KmpCookie> =
    parseCompatCookies(url.toString(), headers.toMultimap(), ::KmpCookie)

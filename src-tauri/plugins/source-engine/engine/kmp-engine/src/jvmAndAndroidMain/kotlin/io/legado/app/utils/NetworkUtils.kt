package io.legado.app.utils

import okhttp3.internal.publicsuffix.PublicSuffixDatabase

@Suppress("DEPRECATION")
internal actual fun resolveScriptUrl(base: URL, relative: String): String = java.net.URL(base, relative).toString()

internal actual fun registrableDomain(host: String): String =
    PublicSuffixDatabase.get().getEffectiveTldPlusOne(host) ?: host

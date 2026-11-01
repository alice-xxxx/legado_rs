package io.legado.app.utils

internal actual fun resolveScriptUrl(base: URL, relative: String): String =
    ScriptUrlCompat.resolveRelative(base.toString(), relative.trim())

internal actual fun registrableDomain(host: String): String = host

package io.legado.app.utils

/** Ksoup 的通用解析器不保留 JDK 的查询相对路径、空路径及越界点段语义。 */
internal object ScriptUrlCompat {
    fun parse(url: String, baseUrl: String?): ScriptUrlParts =
        parse(if (baseUrl.isNullOrEmpty()) url else resolveRelative(baseUrl, url))

    private fun parse(url: String): ScriptUrlParts {
        val schemeEnd = url.indexOf("://")
        if (schemeEnd < 0) {
            return ScriptUrlParts("", "", url, null)
        }
        val scheme = url.substring(0, schemeEnd).lowercase()
        val afterScheme = url.substring(schemeEnd + 3)

        val hostEnd = afterScheme.indexOfAny(charArrayOf('/', '?', '#'))
        val authority = if (hostEnd < 0) afterScheme else afterScheme.substring(0, hostEnd)

        val hostPort = authority.substringAfterLast('@')
        val colonIdx = if (hostPort.startsWith("[")) {
            hostPort.indexOf(']').takeIf { it >= 0 }?.plus(1)?.takeIf { it < hostPort.length && hostPort[it] == ':' } ?: -1
        } else hostPort.lastIndexOf(':')
        val hostOnly = if (colonIdx < 0) hostPort else hostPort.substring(0, colonIdx)
        val portStr = if (colonIdx < 0) null else hostPort.substring(colonIdx + 1)
        val port = portStr?.toIntOrNull() ?: -1
        val origin = if (port > 0) "$scheme://$hostOnly:$port" else "$scheme://$hostOnly"

        val afterAuthority = if (hostEnd < 0) "" else afterScheme.substring(hostEnd)
        val queryStart = afterAuthority.indexOf('?')
        val fragStart = afterAuthority.indexOf('#')
        val pathEnd = minOf(
            if (queryStart < 0) afterAuthority.length else queryStart,
            if (fragStart < 0) afterAuthority.length else fragStart,
        )

        val pathname = afterAuthority.substring(0, pathEnd)
        val query = when {
            queryStart < 0 -> null
            fragStart in 0 until queryStart -> null
            fragStart < 0 -> afterAuthority.substring(queryStart + 1)
            else -> afterAuthority.substring(queryStart + 1, fragStart)
        }
        return ScriptUrlParts(hostOnly, origin, pathname, query)
    }

    fun resolveRelative(baseUrl: String, relative: String): String {
        if (relative.startsWith("http://", true) || relative.startsWith("https://", true)) {
            return relative
        }
        val baseScheme = baseUrl.substringBefore("://", "")
        if (baseScheme.isEmpty()) return relative
        val baseAfterScheme = baseUrl.substringAfter("://", "")
        val baseHostEnd = baseAfterScheme.indexOfAny(charArrayOf('/', '?', '#'))
        val baseAuthority = if (baseHostEnd < 0) baseAfterScheme else baseAfterScheme.substring(0, baseHostEnd)

        val baseFile = if (baseHostEnd < 0) "" else baseAfterScheme.substring(baseHostEnd)
        val basePath = baseFile.substringBefore('?').substringBefore('#')
        return when {
            relative.startsWith("//") -> "$baseScheme:$relative"
            relative.startsWith("/") -> "$baseScheme://$baseAuthority$relative"
            relative.startsWith("?") -> {
                val ind = basePath.lastIndexOf('/')
                val dir = if (ind < 0) "/" else basePath.substring(0, ind) + "/"
                "$baseScheme://$baseAuthority$dir$relative"
            }

            relative.startsWith("#") -> baseUrl.substringBefore('#') + relative
            relative.isEmpty() -> baseUrl
            else -> {
                val pathSpec = relative.substringBefore('?').substringBefore('#')
                val suffix = relative.substring(pathSpec.length)
                val ind = basePath.lastIndexOf('/')

                val baseDir = if (ind >= 0) basePath.substring(
                    0,
                    ind + 1
                ) else if (baseAuthority.isNotEmpty()) "/" else ""
                val joined = "$baseDir$pathSpec"
                val path = if (basePath.isNotEmpty()) normalizeRelativePath(joined) else joined
                "$baseScheme://$baseAuthority$path$suffix"
            }
        }
    }

    private fun normalizeRelativePath(path: String): String {
        var p = path

        var i = p.indexOf("/./")
        while (i >= 0) {
            p = p.substring(0, i) + p.substring(i + 2)
            i = p.indexOf("/./")
        }

        i = 0
        while (i >= 0) {
            i = p.indexOf("/../", i)
            if (i < 0) break
            val prev = if (i > 0) p.lastIndexOf('/', i - 1) else -1
            if (prev >= 0 && p.indexOf("/../", prev) != 0) {
                p = p.substring(0, prev) + p.substring(i + 3)
                i = 0
            } else {
                i += 3
            }
        }

        while (p.endsWith("/..")) {
            val j = p.indexOf("/..")
            val prev = p.lastIndexOf('/', j - 1)
            if (prev >= 0) {
                p = p.substring(0, prev + 1)
            } else {
                break
            }
        }

        if (p.startsWith("./") && p.length > 2) {
            p = p.substring(2)
        }

        if (p.endsWith("/.")) {
            p = p.substring(0, p.length - 1)
        }
        return p
    }

}

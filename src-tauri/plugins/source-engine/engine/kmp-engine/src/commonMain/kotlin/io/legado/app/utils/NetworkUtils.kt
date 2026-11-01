package io.legado.app.utils

import io.legado.app.constant.AppLog
import io.legado.app.help.coroutine.printStackTraceOnDebug

object NetworkUtils {
    fun encodedQuery(str: String): Boolean = NetworkValueChecks.encodedQuery(str)
    fun encodedForm(str: String): Boolean = NetworkValueChecks.encodedForm(str)

    fun getAbsoluteURL(baseURL: String?, relativePath: String): String {
        if (baseURL.isNullOrEmpty() || baseURL.isDataUrl()) return relativePath.trim()
        val parsed = try { URL(baseURL.substringBefore(',')) } catch (error: Exception) {
            error.printStackTraceOnDebug()
            null
        }
        return getAbsoluteURL(parsed, relativePath)
    }

    fun getAbsoluteURL(baseURL: URL?, relativePath: String): String {
        val trimmed = relativePath.trim()
        if (baseURL == null || trimmed.isAbsUrl() || trimmed.isDataUrl()) return trimmed
        if (trimmed.startsWith("javascript")) return ""
        return try { resolveScriptUrl(baseURL, relativePath) } catch (error: Exception) {
            AppLog.put("网址拼接出错\n" + error.message, error)
            trimmed
        }
    }

    fun getBaseUrl(url: String?): String? {
        if (url == null || !(url.startsWith("http://", true) || url.startsWith("https://", true))) return null
        val index = url.indexOf('/', 9)
        return if (index < 0) url else url.take(index)
    }

    fun getSubDomain(url: String): String {
        val base = getBaseUrl(url) ?: return url
        return runCatching { subDomain(base) }.getOrDefault(base)
    }

    fun getSubDomainOrNull(url: String): String? {
        val base = getBaseUrl(url) ?: return null
        return runCatching { subDomain(base) }.getOrNull()
    }

    private fun subDomain(base: String): String {
        val host = parseScriptUrl(base, null).host
        return if (isIPAddress(host)) host else registrableDomain(host)
    }

    fun getDomain(url: String): String {
        val base = getBaseUrl(url) ?: return url
        return runCatching { parseScriptUrl(base, null).host }.getOrDefault(base)
    }

    fun isIPv4Address(input: String?): Boolean = NetworkValueChecks.isIPv4Address(input)
    fun isIPv6Address(input: String?): Boolean = NetworkValueChecks.isIPv6Address(input)
    fun isIPAddress(input: String?): Boolean = NetworkValueChecks.isIPAddress(input)
}

internal expect fun resolveScriptUrl(base: URL, relative: String): String
internal expect fun registrableDomain(host: String): String

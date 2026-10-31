package io.legado.app.utils

import io.legado.app.constant.AppLog
import io.legado.app.help.coroutine.printStackTraceOnDebug
import io.legado.app.utils.NetworkUtils.getAbsoluteURL
import io.legado.app.utils.NetworkUtils.getLocalIPAddress
import okhttp3.internal.publicsuffix.PublicSuffixDatabase
import java.net.InetAddress
import java.net.NetworkInterface
import java.net.SocketException
import java.net.URL
import java.util.Enumeration

/**
 * NetworkUtils 的 JVM 半区 actual (android + jvm 共用)。
 *
 * 详见 commonMain/utils/NetworkUtils.kt expect 注释。
 * 实现逻辑与原 jvmAndAndroidMain 直接实现一致, 行为不变。
 *
 * 附加成员 (仅 jvmAndAndroid 可见, common 无 JDK 类型):
 * - [getAbsoluteURL] (java.net.URL?, String) 重载
 * - [getLocalIPAddress] 返回 List<java.net.InetAddress>
 *
 * 仅 [isAvailable] 留 app (走 connectivityManager, 见 app 端 NetworkAvailability)。
 *
 * 包名/对象名不变, 消费方 import 零改动。
 */
actual object NetworkUtils {

    actual fun encodedQuery(str: String): Boolean = NetworkValueChecks.encodedQuery(str)
    actual fun encodedForm(str: String): Boolean = NetworkValueChecks.encodedForm(str)

    @Suppress("DEPRECATION")
    actual fun getAbsoluteURL(baseURL: String?, relativePath: String): String {
        if (baseURL.isNullOrEmpty() || baseURL.isDataUrl()) return relativePath.trim()
        var absoluteUrl: URL? = null
        try {
            absoluteUrl = URL(baseURL.substringBefore(","))
        } catch (e: Exception) {
            e.printStackTraceOnDebug()
        }
        return getAbsoluteURL(absoluteUrl, relativePath)
    }

    /**
     * 获取绝对地址
     *
     * 附加成员: 参数为 java.net.URL (原版 NetworkUtils.getAbsoluteURL(URL?, String) 重载,
     * 不经 String 门面, 避免 substringBefore(",") 截断含逗号的 URL)。
     */
    @Suppress("DEPRECATION")
    actual fun getAbsoluteURL(baseURL: URL?, relativePath: String): String {
        val relativePathTrim = relativePath.trim()
        if (baseURL == null) return relativePathTrim
        if (relativePathTrim.isAbsUrl()) return relativePathTrim
        if (relativePathTrim.isDataUrl()) return relativePathTrim
        if (relativePathTrim.startsWith("javascript")) return ""
        var relativeUrl = relativePathTrim
        try {
            val parseUrl = URL(baseURL, relativePath)
            relativeUrl = parseUrl.toString()
            return relativeUrl
        } catch (e: Exception) {
            AppLog.put("网址拼接出错\n${e.localizedMessage}", e)
        }
        return relativeUrl
    }

    actual fun getBaseUrl(url: String?): String? {
        url ?: return null
        if (url.startsWith("http://", true)
            || url.startsWith("https://", true)
        ) {
            val index = url.indexOf("/", 9)
            return if (index == -1) {
                url
            } else url.take(index)
        }
        return null
    }

    /**
     * 获取域名，供cookie保存和读取，处理失败返回传入的url
     * http://1.2.3.4 => 1.2.3.4
     * https://www.example.com =>  example.com
     * http://www.biquge.com.cn => biquge.com.cn
     * http://www.content.example.com => example.com
     */
    @Suppress("DEPRECATION")
    actual fun getSubDomain(url: String): String {
        val baseUrl = getBaseUrl(url) ?: return url
        return kotlin.runCatching {
            val mURL = URL(baseUrl)
            val host: String = mURL.host
            //mURL.scheme https/http
            //判断是否为ip
            if (isIPAddress(host)) return host
            //PublicSuffixDatabase处理域名
            PublicSuffixDatabase.get().getEffectiveTldPlusOne(host) ?: host
        }.getOrDefault(baseUrl)
    }

    @Suppress("DEPRECATION")
    actual fun getSubDomainOrNull(url: String): String? {
        val baseUrl = getBaseUrl(url) ?: return null
        return kotlin.runCatching {
            val mURL = URL(baseUrl)
            val host: String = mURL.host
            //mURL.scheme https/http
            //判断是否为ip
            if (isIPAddress(host)) return host
            //PublicSuffixDatabase处理域名
            PublicSuffixDatabase.get().getEffectiveTldPlusOne(host) ?: host
        }.getOrDefault(null)
    }

    @Suppress("DEPRECATION")
    actual fun getDomain(url: String): String {
        val baseUrl = getBaseUrl(url) ?: return url
        return kotlin.runCatching {
            URL(baseUrl).host
        }.getOrDefault(baseUrl)
    }

    /**
     * Get local Ip address.
     *
     * 附加成员: 返回 List<java.net.InetAddress>, common 无 InetAddress 类型, 仅 jvmAndAndroid 暴露。
     */
    fun getLocalIPAddress(): List<InetAddress> {
        val enumeration: Enumeration<NetworkInterface>
        try {
            enumeration = NetworkInterface.getNetworkInterfaces()
        } catch (e: SocketException) {
            e.printStackTraceOnDebug()
            return emptyList()
        }

        val addressList = mutableListOf<InetAddress>()

        while (enumeration.hasMoreElements()) {
            val nif = enumeration.nextElement()
            val addresses = nif.inetAddresses ?: continue
            while (addresses.hasMoreElements()) {
                val address = addresses.nextElement()
                if (!address.isLoopbackAddress && isIPv4Address(address.hostAddress)) {
                    addressList.add(address)
                }
            }
        }
        return addressList
    }

    // 与 hutool RegexPool.IPV4/IPV6 相同的正则，Validator.isIpv4/isIpv6 行为对齐
    actual fun isIPv4Address(input: String?): Boolean = NetworkValueChecks.isIPv4Address(input)
    actual fun isIPv6Address(input: String?): Boolean = NetworkValueChecks.isIPv6Address(input)
    actual fun isIPAddress(input: String?): Boolean = NetworkValueChecks.isIPAddress(input)
}

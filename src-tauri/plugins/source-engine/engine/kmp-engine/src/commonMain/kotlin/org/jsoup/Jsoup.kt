@file:Suppress("unused")

package org.jsoup

import com.fleeksoft.ksoup.Ksoup
import com.fleeksoft.ksoup.nodes.Document
import io.legado.app.help.http.KmpHttpClient
import io.legado.app.utils.URL
import org.jsoup.internal.HttpConnection
import kotlin.concurrent.Volatile
import kotlin.jvm.JvmStatic

/** Jsoup 脚本兼容入口：HTML 由 Ksoup 解析，HTTP 复用宿主传输。 */
object Jsoup {
    @Volatile
    var clientFactory: (() -> KmpHttpClient)? = null

    @JvmStatic
    fun connect(url: String): Connection = HttpConnection().url(url)

    @JvmStatic
    fun connect(url: URL): Connection = HttpConnection().url(url)

    @JvmStatic
    fun newSession(): Connection = HttpConnection()

    @JvmStatic
    fun parse(html: String): Document = Ksoup.parse(html)

    @JvmStatic
    fun parse(html: String, baseUri: String): Document = Ksoup.parse(html, baseUri)

    @JvmStatic
    fun parseBodyFragment(bodyHtml: String): Document = Ksoup.parseBodyFragment(bodyHtml)

    @JvmStatic
    fun parseBodyFragment(bodyHtml: String, baseUri: String): Document = Ksoup.parseBodyFragment(bodyHtml, baseUri)
}

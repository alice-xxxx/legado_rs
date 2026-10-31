package io.legado.app.model.script

import com.fleeksoft.ksoup.nodes.Document
import com.fleeksoft.ksoup.nodes.Element
import com.fleeksoft.ksoup.nodes.Node
import com.fleeksoft.ksoup.parser.Parser
import com.fleeksoft.ksoup.select.Elements
import io.legado.app.help.JsExtensionsPlatform
import org.jsoup.Connection
import org.jsoup.Jsoup

/** Native 无 JVM 反射；仅将书源使用的类委托给已有 KMP 实现。 */
object JavaClassCompat {
    fun interface Callback { fun call(method: String, args: List<Any?>): Any? }

    private val interfaces = setOf(
        "java.lang.Runnable", "java.util.concurrent.Callable", "java.util.Comparator",
        "java.util.function.Consumer", "java.util.function.BiConsumer",
        "java.util.function.Function", "java.util.function.BiFunction",
        "java.util.function.Predicate", "java.util.function.BiPredicate",
        "java.util.function.Supplier", "java.util.function.UnaryOperator", "java.util.function.BinaryOperator"
    )
    private val classes = listOf(
        "org.jsoup.Jsoup", "org.jsoup.nodes.Element", "org.jsoup.select.Elements",
        "org.jsoup.parser.Parser", "java.lang.String", "java.lang.StringBuilder",
        "org.jsoup.Connection", "org.jsoup.Connection\$Method", "java.lang.Thread"
    ) + JavaCryptoCompat.classes + JavaCollectionsCompat.classes + interfaces

    fun loadClass(name: String): Int = classes.indexOf(name) + 1

    fun isInterface(handle: Int): Boolean = name(handle) in interfaces

    fun newAdapter(handle: Int, implementation: Callback): Callback {
        require(isInterface(handle)) { "JavaAdapter requires a supported interface" }
        return object : Callback {
            override fun call(method: String, args: List<Any?>): Any? = when (method) {
                "toString" -> "JavaAdapter(${name(handle)})"
                "hashCode" -> hashCode()
                "equals" -> this === args.firstOrNull()
                else -> {
                    val result = try { implementation.call("invoke", listOf(method, args)) } catch (_: Throwable) { null }
                    result ?: when (name(handle) to method) {
                        "java.util.Comparator" to "compare" -> 0
                        "java.util.function.Predicate" to "test", "java.util.function.BiPredicate" to "test" -> false
                        else -> null
                    }
                }
            }
        }
    }

    fun construct(handle: Int, args: List<Any?>): Any = when (name(handle)) {
        "org.jsoup.nodes.Element" -> Element(args.string(0))
        "org.jsoup.select.Elements" -> Elements().apply {
            args.forEach { value ->
                when (value) {
                    is Element -> add(value)
                    is List<*> -> value.forEach { add(it as Element) }
                    else -> error("Elements requires Element arguments")
                }
            }
        }
        "java.lang.String" -> when (val value = args.firstOrNull()) {
            null -> ""
            is ByteArray, is List<*> -> {
                val bytes = javaBytes(value)
                val input = if (args.getOrNull(1) is Number && args.getOrNull(2) is Number) bytes.copyOfRange(args.int(1), args.int(1) + args.int(2)) else bytes
                JsExtensionsPlatform.bytesToStr(input, args.drop(1).lastOrNull { it is String } as? String ?: "UTF-8")
            }
            else -> value.toString()
        }
        "java.lang.StringBuilder" -> StringBuilder(args.firstOrNull()?.takeUnless { it is Number }?.toString().orEmpty())
        else -> if (name(handle) in JavaCollectionsCompat.classes) JavaCollectionsCompat.construct(name(handle), args) else JavaCryptoCompat.construct(name(handle), args)
    }

    fun callStatic(handle: Int, method: String, args: List<Any?>): Any? = when (name(handle) to method) {
        "org.jsoup.Jsoup" to "parse" -> Jsoup.parse(args.string(0), args.getOrNull(1)?.toString().orEmpty())
        "org.jsoup.Jsoup" to "parseBodyFragment" -> Jsoup.parseBodyFragment(args.string(0), args.getOrNull(1)?.toString().orEmpty())
        "org.jsoup.Jsoup" to "connect" -> Jsoup.connect(args.string(0))
        "org.jsoup.Jsoup" to "newSession" -> Jsoup.newSession()
        "org.jsoup.parser.Parser" to "unescapeEntities" -> Parser.unescapeEntities(args.string(0), args.getOrNull(1) == true)
        "java.lang.String" to "valueOf" -> args.firstOrNull()?.toString() ?: "null"
        "java.lang.Thread" to "sleep" -> {
            require(args.size == 1)
            val millis = (args[0] as Number).toLong()
            require(millis >= 0)
            io.legado.app.utils.platformSleep(millis)
            null
        }
        else -> if (name(handle) == "java.util.Collections") JavaCollectionsCompat.callStatic(method, args)
        else JavaCryptoCompat.callStatic(name(handle), method, args)
    }

    fun field(handle: Int, field: String): Any? = if (name(handle) == "org.jsoup.Connection\$Method") {
        Connection.Method.entries.firstOrNull { it.name == field }
    } else JavaCryptoCompat.field(name(handle), field)

    fun supports(value: Any?): Boolean = value is Callback || value is Node || value is Elements || value is StringBuilder || value is Connection || value is Connection.Response || value is Connection.Method || value is JavaCryptoCompat.Value || JavaCollectionsCompat.supports(value)
    fun isCollection(value: Any?): Boolean = value is Elements || value is JavaCollectionsCompat.Items

    fun call(value: Any, method: String, args: List<Any?>): Any? {
        if (value is Callback) return value.call(method, args)
        if (value is JavaCryptoCompat.Value) return JavaCryptoCompat.call(value, method, args)
        if (JavaCollectionsCompat.supports(value)) return JavaCollectionsCompat.call(value, method, args)
        if (value is Connection.Method) return when (method) {
            "name", "toString" -> value.name
            "ordinal" -> value.ordinal
            else -> error("Unsupported HTTP method enum operation: $method")
        }
        if (value is Elements) return when (method) {
            "size" -> value.size
            "isEmpty" -> value.isEmpty()
            "get" -> value[args.int(0)]
            "first" -> value.firstOrNull()
            "last" -> value.lastOrNull()
            "add" -> value.add(args[0] as Element)
            "addAll" -> value.addAll((args[0] as List<*>).map { it as Element })
            "select" -> value.select(args.string(0))
            "eq" -> value.eq(args.int(0))
            "text" -> value.text()
            "html" -> value.html()
            "outerHtml", "toString" -> value.outerHtml()
            "attr" -> if (args.size == 1) value.attr(args.string(0)) else value.attr(args.string(0), args.string(1))
            "remove" -> value.remove()
            "clear" -> { value.deselectAll(); null }
            "toArray", "toJSON" -> value.map { if (method == "toJSON") it.outerHtml() else it }
            "forEach" -> { value.forEach { (args[0] as Callback).call("accept", listOf(it)) }; null }
            else -> error("Unsupported Elements method: $method")
        }
        if (value is Node) return when (method) {
            "toString", "outerHtml", "toJSON" -> value.outerHtml()
            "attr" -> if (args.size == 1) value.attr(args.string(0)) else value.attr(args.string(0), args.string(1))
            "hasAttr" -> value.hasAttr(args.string(0))
            "absUrl" -> value.absUrl(args.string(0))
            "baseUri" -> value.baseUri()
            "parent" -> value.parent()
            "clone" -> value.clone()
            "remove" -> { value.remove(); null }
            "text" -> (value as Element).let { if (args.isEmpty()) it.text() else it.text(args.string(0)) }
            "html" -> (value as Element).let { if (args.isEmpty()) it.html() else it.html(args.string(0)) }
            "ownText" -> (value as Element).ownText()
            "select" -> (value as Element).select(args.string(0))
            "selectFirst" -> (value as Element).selectFirst(args.string(0))
            "children" -> (value as Element).children()
            "child" -> (value as Element).child(args.int(0))
            "appendChild" -> (value as Element).appendChild(args[0] as Node)
            "append" -> (value as Element).append(args.string(0))
            "prepend" -> (value as Element).prepend(args.string(0))
            "hasClass" -> (value as Element).hasClass(args.string(0))
            "tagName" -> (value as Element).tagName()
            "id" -> (value as Element).id()
            "className" -> (value as Element).className()
            "value", "val" -> (value as Element).value()
            "body" -> (value as Document).body()
            "head" -> (value as Document).head()
            "title" -> (value as Document).title()
            else -> error("Unsupported DOM method: $method")
        }
        if (value is StringBuilder) return when (method) {
            "append" -> value.append(args.firstOrNull())
            "toString" -> value.toString()
            "length" -> value.length
            else -> error("Unsupported StringBuilder method: $method")
        }
        if (value is Connection) return when (method) {
            "url" -> value.url(args.string(0))
            "userAgent" -> value.userAgent(args.string(0))
            "referrer" -> value.referrer(args.string(0))
            "timeout" -> value.timeout(args.int(0))
            "maxBodySize" -> value.maxBodySize(args.int(0))
            "followRedirects" -> value.followRedirects(args[0] as Boolean)
            "ignoreHttpErrors" -> value.ignoreHttpErrors(args[0] as Boolean)
            "ignoreContentType" -> value.ignoreContentType(args[0] as Boolean)
            "header" -> value.header(args.string(0), args.string(1))
            "headers" -> value.headers(JavaCollectionsCompat.stringMap(args[0]))
            "cookie" -> value.cookie(args.string(0), args.string(1))
            "cookies" -> value.cookies(JavaCollectionsCompat.stringMap(args[0]))
            "data" -> if (args.size == 1) value.data(JavaCollectionsCompat.stringMap(args[0])) else value.data(args.string(0), args.string(1))
            "requestBody" -> value.requestBody(args.string(0))
            "method" -> value.method(args[0] as Connection.Method)
            "execute" -> value.execute()
            "get" -> value.get()
            "post" -> value.post()
            else -> error("Unsupported Jsoup connection method: $method")
        }
        if (value is Connection.Response) return when (method) {
            "body", "toString" -> value.body()
            "bodyAsBytes" -> value.bodyAsBytes()
            "parse" -> value.parse()
            "statusCode" -> value.statusCode()
            "statusMessage" -> value.statusMessage()
            "url" -> value.url()?.toString()
            "header" -> if (args.size == 1) value.header(args.string(0)) else value.header(args.string(0), args.string(1))
            "cookie" -> if (args.size == 1) value.cookie(args.string(0)) else value.cookie(args.string(0), args.string(1))
            "headers" -> value.headers()
            "cookies" -> value.cookies()
            "contentType" -> value.contentType()
            "charset" -> if (args.isEmpty()) value.charset() else value.charset(args.string(0))
            "method" -> value.method()
            else -> error("Unsupported Jsoup response method: $method")
        }
        error("Unsupported Java object: $value")
    }

    private fun name(handle: Int): String = classes.getOrNull(handle - 1) ?: error("Unknown Java class handle: $handle")
    private fun List<Any?>.string(index: Int): String = get(index)?.toString() ?: "null"
    private fun List<Any?>.int(index: Int): Int = (get(index) as Number).toInt()
}

internal fun javaBytes(value: Any?): ByteArray = when (value) {
        is ByteArray -> value
        is List<*> -> ByteArray(value.size) { (value[it] as Number).toInt().toByte() }
        is String -> value.encodeToByteArray()
        else -> error("Expected byte array")
}

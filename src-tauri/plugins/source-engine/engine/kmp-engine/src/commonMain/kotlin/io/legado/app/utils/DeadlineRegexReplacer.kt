package io.legado.app.utils

import io.legado.app.exception.NoStackTraceException
import io.legado.app.exception.RegexTimeoutException
import io.legado.app.model.script.JsBindings
import io.legado.app.model.script.JsEngines
import kotlin.time.Duration.Companion.milliseconds
import kotlin.time.TimeSource

/** 同步中止匹配；各平台都在正则读取字符时检查期限，不遗留后台匹配任务。 */
object DeadlineRegexReplacer : RegexReplacer {
    override fun replace(source: CharSequence, regex: Regex, replacement: String, timeout: Long): String {
        val input = DeadlineInput(source, timeout)
        try {
            input.checkDeadline()
            val matches = regex.findAll(input).iterator()
            if (!matches.hasNext()) return source.toString()
            val engine = if (replacement.startsWith("@js:")) JsEngines.get() else null
            val scope = engine?.getRuntimeScope(JsBindings())
            try {
                val compiled = if (engine != null && scope != null) engine.compile(engine.wrapJsForEval(replacement.substring(4)), scope) else null
                val result = buildString(source.length) {
                    var end = 0
                    do {
                        input.checkDeadline()
                        val match = matches.next()
                        append(source, end, match.range.first)
                        if (compiled != null && engine != null && scope != null) {
                            engine.injectBindings(scope, JsBindings().apply { this["result"] = match.value })
                            append(compiled.eval(scope, null)?.toString().orEmpty())
                        } else append(expandGroups(match, replacement))
                        end = match.range.last + 1
                    } while (matches.hasNext())
                    append(source, end, source.length)
                }
                input.checkDeadline()
                return result
            } finally {
                scope?.close()
            }
        } catch (_: RegexDeadlineException) {
            val message = "替换超时\n替换规则$regex\n替换内容:$source"
            val exception = RegexTimeoutException(message)
            RegexErrorHandlers.getOrNull()?.apply {
                onTimeoutToast(message)
                saveCrashInfo(exception)
            }
            throw exception
        }
    }

    /** 统一 Java 的 $n / ${name} 与反斜杠转义规则，包括尾部空匹配。 */
    private fun expandGroups(match: MatchResult, replacement: String): String = buildString {
        var index = 0
        while (index < replacement.length) {
            when (val char = replacement[index++]) {
                '\\' -> {
                    require(index < replacement.length) { "Trailing replacement escape" }
                    append(replacement[index++])
                }
                '$' -> {
                    require(index < replacement.length) { "Missing replacement group" }
                    if (replacement[index] == '{') {
                        val end = replacement.indexOf('}', ++index)
                        require(end > index) { "Invalid named replacement group" }
                        append(match.groups[replacement.substring(index, end)]?.value.orEmpty())
                        index = end + 1
                    } else {
                        require(replacement[index] in '0'..'9') { "Invalid replacement group" }
                        var group = replacement[index++].digitToInt()
                        require(group < match.groups.size) { "Unknown replacement group: $group" }
                        while (index < replacement.length && replacement[index] in '0'..'9') {
                            val next = group.toLong() * 10 + replacement[index].digitToInt()
                            if (next >= match.groups.size) break
                            group = next.toInt()
                            index++
                        }
                        append(match.groups[group]?.value.orEmpty())
                    }
                }
                else -> append(char)
            }
        }
    }
}

private class RegexDeadlineException : NoStackTraceException("regex deadline")

private class DeadlineInput(private val source: CharSequence, timeout: Long) : CharSequence {
    private val start = TimeSource.Monotonic.markNow()
    private val limit = timeout.milliseconds
    private var reads = 0

    fun checkDeadline() {
        if (start.elapsedNow() >= limit) throw RegexDeadlineException()
    }

    override val length: Int get() = source.length
    override fun get(index: Int): Char {
        if ((reads++ and 0xFFF) == 0) checkDeadline()
        return source[index]
    }
    override fun subSequence(startIndex: Int, endIndex: Int): CharSequence {
        checkDeadline()
        return source.subSequence(startIndex, endIndex)
    }
    override fun toString(): String { checkDeadline(); return source.toString() }
}

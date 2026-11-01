package com.script.quickjs

import io.legado.app.help.CommonLruCache

/** Exact class names and package prefixes, with the original dot-boundary rule. */
class ClassNameMatcher(classNames: List<String>) {
    private val names = classNames.sorted()
    private val cache = CommonLruCache<String, Boolean>(64)

    fun match(className: String): Boolean {
        cache[className]?.let { return it }
        val index = names.binarySearch(className)
        val matches = index >= 0 || names.getOrNull(-index - 2)?.let { prefix ->
            className.getOrNull(prefix.length) == '.' && className.startsWith(prefix)
        } == true
        cache.put(className, matches)
        return matches
    }
}

package io.legado.app.model.script

/** Java 集合接口复用 Kotlin 集合；Map 的 keys/values 保持为原集合的视图。 */
internal object JavaCollectionsCompat {
    val classes = listOf("java.util.ArrayList", "java.util.HashMap", "java.util.LinkedHashMap")
    sealed interface Value
    class Items(val values: MutableCollection<Any?>) : Value
    private class Entries(val values: MutableMap<Any?, Any?>) : Value
    private class Cursor(val iterator: MutableIterator<Any?>) : Value

    fun supports(value: Any?): Boolean = value is Value || value is MutableMap.MutableEntry<*, *>

    fun construct(name: String, args: List<Any?>): Value {
        val initial = args.firstOrNull()
        if (initial is Number) require(initial.toInt() >= 0) { "Negative collection capacity" }
        return when (name) {
            "java.util.ArrayList" -> Items(when (initial) {
                null, is Number -> arrayListOf()
                is Items -> ArrayList(initial.values)
                is Collection<*> -> ArrayList(initial)
                else -> error("ArrayList requires a collection")
            })
            "java.util.HashMap", "java.util.LinkedHashMap" -> Entries(when (initial) {
                null, is Number -> linkedMapOf()
                is Entries -> LinkedHashMap(initial.values)
                is Map<*, *> -> LinkedHashMap(initial)
                else -> error("HashMap requires a map")
            })
            else -> error("Unsupported Java constructor: $name")
        }
    }

    fun stringMap(value: Any?): Map<String, String> {
        val map = if (value is Entries) value.values else value as Map<*, *>
        return map.entries.associate { (key, item) -> (key as String) to (item as String) }
    }

    fun call(value: Any, method: String, args: List<Any?>): Any? = when (value) {
        is Items -> with(value.values) {
            when (method) {
                "size" -> size
                "isEmpty" -> isEmpty()
                "contains" -> contains(args[0])
                "get" -> (this as List<*>)[args.int(0)]
                "set" -> (this as MutableList<Any?>).set(args.int(0), args[1])
                "add" -> if (args.size == 1) add(args[0]) else { (this as MutableList<Any?>).add(args.int(0), args[1]); null }
                "addAll" -> addAll(if (args[0] is Items) (args[0] as Items).values else args[0] as Collection<Any?>)
                "remove" -> if (this is MutableList<*> && args[0] is Number) removeAt(args.int(0)) else remove(args[0])
                "clear" -> { clear(); null }
                "iterator" -> Cursor(iterator())
                "toArray", "toJSON" -> toList()
                "toString" -> toString()
                else -> error("Unsupported collection method: $method")
            }
        }
        is Entries -> with(value.values) {
            when (method) {
                "size" -> size
                "isEmpty" -> isEmpty()
                "get" -> get(args[0])
                "put" -> put(args[0], args[1])
                "containsKey" -> containsKey(args[0])
                "containsValue" -> containsValue(args[0])
                "remove" -> remove(args[0])
                "clear" -> { clear(); null }
                "keySet" -> Items(keys)
                "values" -> Items(values)
                "entrySet" -> Items(entries as MutableCollection<Any?>)
                "toJSON" -> entries.associate { it.key.toString() to it.value }
                "toString" -> toString()
                else -> error("Unsupported map method: $method")
            }
        }
        is Cursor -> when (method) {
            "hasNext" -> value.iterator.hasNext()
            "next" -> value.iterator.next()
            "remove" -> { value.iterator.remove(); null }
            else -> error("Unsupported iterator method: $method")
        }
        is MutableMap.MutableEntry<*, *> -> when (method) {
            "getKey" -> value.key
            "getValue" -> value.value
            "setValue" -> (value as MutableMap.MutableEntry<Any?, Any?>).setValue(args[0])
            else -> error("Unsupported entry method: $method")
        }
        else -> error("Unsupported Java collection: $value")
    }

    private fun List<Any?>.int(index: Int) = (get(index) as Number).toInt()
}

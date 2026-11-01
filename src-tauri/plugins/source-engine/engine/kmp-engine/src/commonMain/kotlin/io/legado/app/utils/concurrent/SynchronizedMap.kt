package io.legado.app.utils.concurrent

import kotlinx.atomicfu.locks.SynchronizedObject
import kotlinx.atomicfu.locks.synchronized

private class SnapshotEntry<K, V>(
    override val key: K,
    value: V
) : MutableMap.MutableEntry<K, V> {
    private var current: V = value

    override val value: V get() = current

    override fun setValue(newValue: V): V {
        val old = current
        current = newValue
        return old
    }
}

internal fun <K, V> newSynchronizedMap(): MutableMap<K, V> {
    val lock = SynchronizedObject()
    val delegate = mutableMapOf<K, V>()
    return object : MutableMap<K, V> {
        override val size: Int get() = synchronized(lock) { delegate.size }
        override fun isEmpty(): Boolean = synchronized(lock) { delegate.isEmpty() }
        override fun containsKey(key: K): Boolean = synchronized(lock) { delegate.containsKey(key) }
        override fun containsValue(value: V): Boolean = synchronized(lock) { delegate.containsValue(value) }
        override fun get(key: K): V? = synchronized(lock) { delegate[key] }
        override val keys: MutableSet<K>
            get() = synchronized(lock) { delegate.keys.toMutableSet() }
        override val values: MutableCollection<V>
            get() = synchronized(lock) { delegate.values.toMutableList() }
        override val entries: MutableSet<MutableMap.MutableEntry<K, V>>
            get() = synchronized(lock) {
                delegate.entries.mapTo(LinkedHashSet()) { SnapshotEntry(it.key, it.value) }
            }
        override fun put(key: K, value: V): V? = synchronized(lock) { delegate.put(key, value) }
        override fun remove(key: K): V? = synchronized(lock) { delegate.remove(key) }
        override fun putAll(from: Map<out K, V>) = synchronized(lock) { delegate.putAll(from) }
        override fun clear() = synchronized(lock) { delegate.clear() }
    }
}

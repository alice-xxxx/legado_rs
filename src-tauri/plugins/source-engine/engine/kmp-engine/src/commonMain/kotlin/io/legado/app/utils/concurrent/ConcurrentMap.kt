package io.legado.app.utils.concurrent

/** Platform concurrent map; Native uses the shared lock and snapshot fallback. */
expect fun <K, V> newConcurrentMap(): MutableMap<K, V>

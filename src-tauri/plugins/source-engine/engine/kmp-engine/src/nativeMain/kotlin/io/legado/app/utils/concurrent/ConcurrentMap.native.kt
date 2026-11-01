package io.legado.app.utils.concurrent

actual fun <K, V> newConcurrentMap(): MutableMap<K, V> = newSynchronizedMap()

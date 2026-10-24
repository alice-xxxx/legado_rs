package io.legado.sourceengine.bridge

import kotlinx.coroutines.runBlocking

/** Synchronous script APIs use the storage port bound to the current execution. */
internal object EngineStorage {
    private fun <T> call(block: suspend HostStorage.() -> T): T = runBlocking {
        requireNotNull(SourceEngineHostRegistry.current()) { "No active source host" }.storage.block()
    }

    fun get(namespace: String, key: String): String? = call { get(namespace, key) }
    fun put(namespace: String, key: String, value: String, ttlSeconds: Long = 0) =
        call { put(namespace, key, value, ttlSeconds) }
    fun delete(namespace: String, key: String) = call { delete(namespace, key) }
    fun deletePrefixes(namespace: String, prefixes: List<String>) = call { deletePrefixes(namespace, prefixes) }
    fun deleteContains(namespace: String, value: String) = call { deleteContains(namespace, value) }
    fun clear(namespace: String) = call { clear(namespace) }
}

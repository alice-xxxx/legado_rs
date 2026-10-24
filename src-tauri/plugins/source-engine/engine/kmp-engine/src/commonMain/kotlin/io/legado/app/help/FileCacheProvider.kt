package io.legado.app.help

import kotlin.concurrent.Volatile

object FileCacheProviders {
    @Volatile
    var impl: FileCacheProvider? = null

    fun get(): FileCacheProvider = impl ?: error("FileCacheProvider not registered")
}

interface FileCacheProvider {
    /** 写入字符串缓存, saveTime 单位秒, 0 表示永久。 */
    fun put(key: String, value: String, saveTime: Int = 0, persistent: Boolean = false)

    /** 读取字符串缓存, 不存在或已过期返回 null。 */
    fun getAsString(key: String, persistent: Boolean = false): String?

    /** 写入二进制缓存, saveTime 单位秒, 0 表示永久。 */
    fun put(key: String, value: ByteArray, saveTime: Int = 0, persistent: Boolean = false)

    /** 读取二进制缓存, 不存在或已过期返回 null。 */
    fun getAsBinary(key: String, persistent: Boolean = false): ByteArray?

    /** 删除指定 key 的缓存。 */
    fun remove(key: String, persistent: Boolean = false)
}

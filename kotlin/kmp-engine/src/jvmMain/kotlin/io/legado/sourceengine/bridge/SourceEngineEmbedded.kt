package io.legado.sourceengine.bridge

import io.legado.app.help.coroutine.registerJvmDebugState
import io.legado.app.utils.KS_JSON
import io.legado.sourceengine.bridge.http.RustHostHttpProvider
import io.legado.sourceengine.bridge.HostHttpRequest
import io.legado.sourceengine.bridge.HostHttpResponse
import io.legado.sourceengine.bridge.HostStorage
import io.legado.sourceengine.bridge.SourceEngineCall
import io.legado.sourceengine.bridge.SourceEngineHost
import io.legado.sourceengine.bridge.WebBookSourceRuleExecutor
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put

/**
 * 桌面 Rust 进程内嵌 JVM 后调用的稳定入口。
 *
 * 同步 JNI 回调把网络与存储操作交给 Rust Host。初始化只做一次，因为 JVM 不能安全销毁后重建；
 * Rust 串行化请求，以管理进程级 Provider/QuickJS 状态。
 */
object SourceEngineEmbedded {
    @Volatile
    private var initialized = false

    @JvmStatic
    @Synchronized
    fun executeJson(requestJson: String): String {
        return runCatching {
            initializeOnce()
            val request = KS_JSON.parseToJsonElement(requestJson) as JsonObject
            runBlocking { WebBookSourceRuleExecutor.execute(request.toCall(), EmbeddedRustSourceEngineHost) }
        }.getOrElse { error ->
            // JNI 边界不传播 JVM 异常对象；返回结构化错误，让 Rust 保留 Kotlin 异常链用于诊断。
            buildJsonObject { put("__sourceEngineError", error.stackTraceToString()) }.toString()
        }
    }

    @Synchronized
    private fun initializeOnce() {
        if (initialized) return
        System.setProperty("java.awt.headless", "true")
        System.setProperty("file.encoding", "UTF-8")
        registerJvmDebugState(false)
        RustHostHttpProvider.installNative()
        RustSourceEngineProviders.install()
        initialized = true
    }
}

private fun JsonObject.toCall() = SourceEngineCall(
    operation = requiredString("operation"),
    sourceJson = required("source").toString(),
    keyword = this["keyword"]?.jsonPrimitive?.contentOrNull,
    page = this["page"]?.jsonPrimitive?.intOrNull,
    bookJson = this["book"]?.takeUnless { it == JsonNull }?.toString(),
    chapterJson = this["chapter"]?.takeUnless { it == JsonNull }?.toString(),
    nextChapterUrl = this["nextChapterUrl"]?.jsonPrimitive?.contentOrNull,
)

private fun JsonObject.required(name: String) =
    this[name] ?: error("Missing required source-engine field: $name")

private fun JsonObject.requiredString(name: String) =
    required(name).jsonPrimitive.contentOrNull ?: error("Source-engine field '$name' must be a string")

/** 桌面实现将 KMP 宿主请求转成 JNI RPC；Rust 继续拥有网络、Cookie Jar 和宿主数据目录。 */
private object EmbeddedRustSourceEngineHost : SourceEngineHost {
    override val storage: HostStorage = object : HostStorage {
        override suspend fun get(namespace: String, key: String): String? =
            RustHostHttpProvider.storage(namespace, "get", key)

        override suspend fun put(namespace: String, key: String, value: String, ttlSeconds: Long?) {
            RustHostHttpProvider.storage(
                namespace = namespace,
                operation = "put",
                key = key,
                value = value,
                ttlSeconds = ttlSeconds?.coerceAtMost(Int.MAX_VALUE.toLong())?.toInt(),
            )
        }

        override suspend fun delete(namespace: String, key: String) {
            RustHostHttpProvider.storage(namespace, "delete", key)
        }

        override suspend fun deletePrefixes(namespace: String, prefixes: List<String>) {
            RustHostHttpProvider.storage(namespace, "deletePrefixes", keyPrefixes = prefixes)
        }

        override suspend fun deleteContains(namespace: String, value: String) {
            RustHostHttpProvider.storage(namespace, "deleteContains", contains = value)
        }

        override suspend fun clear(namespace: String) {
            RustHostHttpProvider.storage(namespace, "clear")
        }
    }

    override suspend fun executeHttp(request: HostHttpRequest): HostHttpResponse =
        RustHostHttpProvider.executeHostHttp(request)
}

@file:Suppress("unused")

package io.legado.app.help.source

import io.legado.app.data.entities.BookSource
import io.legado.app.data.entities.rule.ExploreKind
import io.legado.app.data.entities.rule.FlexChildStyle
import io.legado.app.data.entities.rule.RowUi
import io.legado.app.help.ExploreKindsCacheProviders
import io.legado.app.help.coroutine.printStackTraceOnDebug
import io.legado.app.model.script.runScriptWithContext
import io.legado.app.utils.GSON
import io.legado.app.utils.MD5Utils
import io.legado.app.utils.fromJsonArray
import io.legado.app.utils.isJsonArray
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import io.legado.app.utils.concurrent.newConcurrentMap
import kotlinx.atomicfu.locks.SynchronizedObject
import kotlinx.atomicfu.locks.synchronized

// 发现分类及缓存沿用引擎入口的宿主和协程通道。

private val mutexMap by lazy { hashMapOf<String, Mutex>() }
private val mutexMapLock = SynchronizedObject()

internal val exploreKindsMap by lazy { newConcurrentMap<String, List<ExploreKind>>() }

private fun BookSource.getExploreKindsKey(): String {
    return MD5Utils.md5Encode(bookSourceUrl + exploreUrl)
}

suspend fun BookSource.exploreKinds(): List<ExploreKind> {
    val exploreKindsKey = getExploreKindsKey()
    exploreKindsMap[exploreKindsKey]?.let { return it }
    val exploreUrl = exploreUrl
    if (exploreUrl.isNullOrBlank()) {
        return emptyList()
    }
    val mutex = synchronized(mutexMapLock) { mutexMap.getOrPut(bookSourceUrl) { Mutex() } }
    mutex.withLock {
        exploreKindsMap[exploreKindsKey]?.let { return it }
        val kinds = arrayListOf<ExploreKind>()
        kotlin.runCatching {
            var ruleStr = exploreUrl
            if (exploreUrl.startsWith("<js>", true)
                || exploreUrl.startsWith("@js:", true)
            ) {
                ruleStr = ExploreKindsCacheProviders.impl?.getAsString(exploreKindsKey)
                if (ruleStr.isNullOrBlank()) {
                    val jsStr = if (exploreUrl.startsWith("@")) {
                        exploreUrl.substring(4)
                    } else {
                        exploreUrl.substring(4, exploreUrl.lastIndexOf("<"))
                    }
                    ruleStr = runScriptWithContext {
                        // 发现控件的值按源独立保存，方法不参与 JSON 序列化。
                        val setup = """
                            globalThis.infoMap = JSON.parse(source.get('exploreInfoMap') || '{}');
                            Object.defineProperties(infoMap, {
                                get: {value: function(key) { return this[key]; }},
                                put: {value: function(key, value) { var old = this[key]; this[key] = value; return old; }},
                                save: {value: function() { source.put('exploreInfoMap', JSON.stringify(this)); }}
                            });
                        """.trimIndent()
                        evalJS(setup+"\n"+jsStr).toString().trim()
                    }
                    ExploreKindsCacheProviders.impl?.put(exploreKindsKey, ruleStr)
                }
            }
            if (ruleStr.isJsonArray()) {
                GSON.fromJsonArray<ExploreKind>(io.legado.app.model.script.RhinoSyntax.json(ruleStr)).getOrThrow().let {
                    kinds.addAll(it)
                }
            } else {
                ruleStr.split("(&&|\n)+".toRegex()).forEach { kindStr ->
                    val kindCfg = kindStr.split("::")
                    var title = kindCfg.first()
                    var type = RowUi.Type.text
                    var cols: Int? = null
                    when {
                        title.startsWith("BUTTON:") -> {
                            title = title.substring(7)
                            type = RowUi.Type.button
                        }

                        title.startsWith("TITLE:") -> {
                            title = title.substring(6)
                            type = RowUi.Type.title
                            cols = 1
                        }
                    }
                    kinds.add(
                        ExploreKind(
                            title,
                            type,
                            kindCfg.getOrNull(1),
                            cols?.let { FlexChildStyle(cols = it) }
                        )
                    )
                }
            }
        }.onFailure {
            kinds.add(ExploreKind("ERROR:${it.message}", url = it.stackTraceToString()))
            it.printStackTraceOnDebug()
        }
        exploreKindsMap[exploreKindsKey] = kinds
        return kinds
    }
}

suspend fun BookSource.clearExploreKindsCache() {
    val exploreKindsKey = getExploreKindsKey()
    exploreKindsMap.remove(exploreKindsKey)
    ExploreKindsCacheProviders.impl?.remove(exploreKindsKey)
}

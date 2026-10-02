package com.script.quickjs

// 抽取后按 JVM/Android source set 分别编译，不再依赖 QuickJS 模块的 common expect/actual 组合。

import androidx.collection.LongSparseArray
import androidx.collection.LruCache

/**
 * Android actual: 委托到 androidx.collection 实现, 性能与原行为完全一致。
 *
 * 注: 不能用 `actual typealias` 因为 LongSparseArray 是 final class 无法继承,
 * 改用组合委托模式 (delegate), 暴露与 expect 一致的 API 子集。
 */
class LongSparseArrayCompat<T> constructor() {
    private val delegate = LongSparseArray<T>()

    fun put(key: Long, value: T) {
        delegate.put(key, value)
    }

    operator fun get(key: Long): T? = delegate.get(key)

    fun remove(key: Long) {
        delegate.remove(key)
    }

    fun clear() {
        delegate.clear()
    }
}

class LruCacheCompat<K : Any, V : Any> constructor(maxSize: Int) {
    private val delegate = LruCache<K, V>(maxSize)

    operator fun get(key: K): V? = delegate.get(key)

    fun put(key: K, value: V) {
        // androidx.collection.LruCache.put 返回旧值 V?, expect 声明返回 Unit, 显式忽略
        delegate.put(key, value)
    }
}


package io.legado.app.utils

/** 使用 Kotlin 标准库生成随机 UUID。 */
@OptIn(kotlin.uuid.ExperimentalUuidApi::class)
fun randomUUIDString(): String = kotlin.uuid.Uuid.random().toString()

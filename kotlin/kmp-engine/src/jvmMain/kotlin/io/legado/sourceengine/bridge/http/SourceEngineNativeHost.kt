@file:JvmName("SourceEngineNativeHost")

package io.legado.sourceengine.bridge.http

/** Rust 在创建嵌入式 JVM 时注册这两个同步宿主回调。 */
external fun nativeHttpRequest(eventJson: String): String

external fun nativeStorageRequest(eventJson: String): String

external fun nativeImageRequest(eventJson: String): String

external fun nativeBrowserSnapshot(requestJson: String): String

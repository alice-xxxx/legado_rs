@file:JvmName("RustSourceEngineNative")

package io.legado.sourceengine.android

/**
 * Small JNI surface for Android. The native library name follows Cargo's `[lib] name`; top-level
 * external methods expose stable static JNI symbols to the Kotlin host adapter.
 */
private val rustSourceEngineLibraryLoaded: Unit = System.loadLibrary("legado_lib")

internal fun rustSourceHttpRequest(requestJson: String): String {
    rustSourceEngineLibraryLoaded
    return nativeHttpRequest(requestJson)
}

internal fun rustSourceStorageRequest(requestJson: String, appDataDir: String): String {
    rustSourceEngineLibraryLoaded
    return nativeStorageRequest(requestJson, appDataDir)
}

private external fun nativeHttpRequest(requestJson: String): String

private external fun nativeStorageRequest(requestJson: String, appDataDir: String): String

package io.legado.app.utils

import okio.buffer

expect class URL(url: String) {
    fun toExternalForm(): String
    override fun toString(): String
}

expect fun URL.urlQuery(): String?

// These map to java.io on JVM; the library provides Native implementations.
typealias InputStream = com.fleeksoft.io.InputStream
typealias Closeable = com.fleeksoft.io.Closeable

fun ByteArray.toInputStream(): InputStream = com.fleeksoft.io.ByteArrayInputStream(this)

fun InputStream.readAllAndClose(): ByteArray = try {
    asSource().buffer().readByteArray()
} finally {
    runCatching { close() }
}

fun Throwable.isSecurityException(): Boolean = this is io.legado.app.exception.SecurityException

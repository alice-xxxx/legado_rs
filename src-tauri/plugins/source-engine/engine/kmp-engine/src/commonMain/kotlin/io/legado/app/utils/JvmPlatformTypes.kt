package io.legado.app.utils

import okio.Buffer

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
    // Avoid InputStream.readAllBytes(), which requires a newer Android API.
    val result = Buffer()
    val bytes = ByteArray(8192)
    while (true) {
        val count = read(bytes)
        if (count <= 0) break
        result.write(bytes, 0, count)
    }
    result.readByteArray()
} finally {
    runCatching { close() }
}

expect class File(path: String)
expect fun Throwable.isSecurityException(): Boolean
expect fun String.platformIntern(): String

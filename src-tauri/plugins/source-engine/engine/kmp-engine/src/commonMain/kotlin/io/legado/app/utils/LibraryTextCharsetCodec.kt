package io.legado.app.utils

import com.fleeksoft.charset.Charsets
import com.fleeksoft.charset.decodeToString
import com.fleeksoft.charset.toByteArray
import kotlinx.atomicfu.locks.SynchronizedObject
import kotlinx.atomicfu.locks.synchronized

private val charsetLock = SynchronizedObject()

internal class LibraryTextCharsetCodec(name: String) : TextCharsetCodec {
    private val charset = synchronized(charsetLock) { Charsets.forName(name) }
    override val name: String = charset.name()
    override fun decode(bytes: ByteArray, offset: Int, length: Int): String =
        bytes.decodeToString(charset, offset, length)
    override fun encode(str: String): ByteArray = str.toByteArray(charset)
}

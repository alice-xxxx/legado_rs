package io.legado.app.help.crypto

import io.legado.app.utils.Base64Lenient
import io.legado.app.utils.InputStream
import io.legado.app.utils.isHex
import io.legado.app.utils.readAllAndClose
import okio.ByteString.Companion.decodeHex

/** 保留 Hutool 的 Hex 优先规则，以及奇数位 Hex 的前置补零。 */
internal fun decodeCipherText(data: String): ByteArray =
    if (data.isNotEmpty() && data.isHex()) {
        (if (data.length % 2 == 0) data else "0$data").decodeHex().toByteArray()
    } else Base64Lenient.decode(data)

internal fun cryptoInputBytes(data: Any, ciphertext: Boolean = false): ByteArray = when (data) {
    is ByteArray -> data
    is String -> if (ciphertext) decodeCipherText(data) else data.encodeToByteArray()
    is InputStream -> data.readAllAndClose()
    else -> throw IllegalArgumentException("Unexpected input type")
}

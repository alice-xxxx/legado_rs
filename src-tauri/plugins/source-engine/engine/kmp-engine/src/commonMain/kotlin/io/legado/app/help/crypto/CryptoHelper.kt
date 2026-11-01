package io.legado.app.help.crypto

import io.legado.app.utils.Base64Lenient

/** Login-info AES format is shared; platforms only perform the cipher operation. */
object CryptoHelper {
    fun decryptAesEcbPkcs5Base64(key: ByteArray, base64Data: String): String =
        aesEcbPkcs5(key, Base64Lenient.decode(base64Data), encrypt = false).decodeToString()

    fun encryptAesEcbPkcs5Base64(key: ByteArray, data: String): String =
        Base64Lenient.encodeToString(aesEcbPkcs5(key, data.encodeToByteArray(), encrypt = true))
}

internal expect fun aesEcbPkcs5(key: ByteArray, data: ByteArray, encrypt: Boolean): ByteArray

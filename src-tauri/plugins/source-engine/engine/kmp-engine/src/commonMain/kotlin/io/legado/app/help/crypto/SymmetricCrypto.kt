package io.legado.app.help.crypto

import io.legado.app.utils.Base64Lenient
import io.legado.app.utils.textCharsetCodec
import io.legado.app.utils.toHexLower

interface SymmetricCrypto {
    fun setIv(iv: ByteArray): SymmetricCrypto
    fun encrypt(data: ByteArray): ByteArray
    fun decrypt(data: ByteArray): ByteArray

    fun encrypt(data: String): ByteArray = encrypt(data.encodeToByteArray())
    fun encryptHex(data: ByteArray): String = encrypt(data).toHexLower()
    fun encryptHex(data: String): String = encrypt(data).toHexLower()
    fun encryptBase64(data: ByteArray): String = Base64Lenient.encodeToString(encrypt(data))
    fun encryptBase64(data: String): String = Base64Lenient.encodeToString(encrypt(data))
    fun encryptBase64(data: String, charset: String?): String =
        encryptBase64(if (charset.isNullOrBlank()) data.encodeToByteArray() else textCharsetCodec(charset).encode(data))

    fun decrypt(data: String): ByteArray = decrypt(decodeCipherText(data))
}

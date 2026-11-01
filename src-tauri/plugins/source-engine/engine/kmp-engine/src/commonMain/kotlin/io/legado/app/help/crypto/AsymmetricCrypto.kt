package io.legado.app.help.crypto

import io.legado.app.utils.Base64Lenient
import io.legado.app.utils.toHexLower

interface AsymmetricCrypto {
    fun setPrivateKey(key: ByteArray): AsymmetricCrypto
    fun setPublicKey(key: ByteArray): AsymmetricCrypto
    fun setPrivateKey(key: String): AsymmetricCrypto = setPrivateKey(key.encodeToByteArray())
    fun setPublicKey(key: String): AsymmetricCrypto = setPublicKey(key.encodeToByteArray())

    fun encryptBytes(data: ByteArray, usePublicKey: Boolean?): ByteArray
    fun decryptBytes(data: ByteArray, usePublicKey: Boolean?): ByteArray

    fun encrypt(data: Any, usePublicKey: Boolean?): ByteArray = encryptBytes(cryptoInputBytes(data), usePublicKey)
    fun decrypt(data: Any, usePublicKey: Boolean?): ByteArray = decryptBytes(cryptoInputBytes(data, ciphertext = true), usePublicKey)
    fun decryptStr(data: Any, usePublicKey: Boolean?): String = decrypt(data, usePublicKey).decodeToString()
    fun encryptHex(data: Any, usePublicKey: Boolean?): String = encrypt(data, usePublicKey).toHexLower()
    fun encryptBase64(data: Any, usePublicKey: Boolean?): String = Base64Lenient.encodeToString(encrypt(data, usePublicKey))
}

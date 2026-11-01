package io.legado.app.help.crypto

import io.legado.app.utils.toHexLower

interface Sign {
    fun setPrivateKey(key: ByteArray): Sign
    fun setPublicKey(key: ByteArray): Sign
    fun setPrivateKey(key: String): Sign = setPrivateKey(key.encodeToByteArray())
    fun setPublicKey(key: String): Sign = setPublicKey(key.encodeToByteArray())

    fun sign(data: ByteArray): ByteArray
    fun verify(data: ByteArray, sign: ByteArray): Boolean
    fun sign(data: String): ByteArray = sign(data.encodeToByteArray())
    fun signHex(data: ByteArray): String = sign(data).toHexLower()
    fun signHex(data: String): String = sign(data).toHexLower()
    fun verify(data: String, sign: ByteArray): Boolean = verify(data.encodeToByteArray(), sign)
}

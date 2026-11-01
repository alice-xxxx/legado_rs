package io.legado.app.help.crypto

import cn.hutool.crypto.CipherWrapper
import cn.hutool.crypto.KeyUtil
import cn.hutool.crypto.asymmetric.KeyType
import javax.crypto.Cipher

@Suppress("unused")
class AsymmetricCryptoAndroid(algorithm: String) : cn.hutool.crypto.asymmetric.AsymmetricCrypto(algorithm), AsymmetricCrypto {

    // Default JCE avoids provider-specific RSA block splitting (weapi encSecKey).
    override fun initCipher() {
        try {
            cipherWrapper = CipherWrapper(Cipher.getInstance(algorithm))
        } catch (e: Exception) {
            super.initCipher()
        }
    }

    @Suppress("MemberVisibilityCanBePrivate")
    override fun setPrivateKey(key: ByteArray): AsymmetricCryptoAndroid {
        setPrivateKey(
            KeyUtil.generatePrivateKey(this.algorithm, key)
        )
        return this
    }

    override fun setPrivateKey(key: String): AsymmetricCryptoAndroid = setPrivateKey(key.encodeToByteArray())

    @Suppress("MemberVisibilityCanBePrivate")
    override fun setPublicKey(key: ByteArray): AsymmetricCryptoAndroid {
        setPublicKey(
            KeyUtil.generatePublicKey(this.algorithm, key)
        )
        return this
    }

    override fun setPublicKey(key: String): AsymmetricCryptoAndroid = setPublicKey(key.encodeToByteArray())

    private fun getKeyType(usePublicKey: Boolean? = true): KeyType {
        return when (usePublicKey) {
            true -> KeyType.PublicKey
            else -> KeyType.PrivateKey
        }
    }

    override fun decryptBytes(data: ByteArray, usePublicKey: Boolean?): ByteArray = decrypt(data, getKeyType(usePublicKey))
    override fun encryptBytes(data: ByteArray, usePublicKey: Boolean?): ByteArray = encrypt(data, getKeyType(usePublicKey))

    // Explicit single-argument overloads are required by the script reflection bridge.
    fun decrypt(data: Any): ByteArray = decrypt(data, true)

    fun decryptStr(data: Any): String = decryptStr(data, true)

    fun encrypt(data: Any): ByteArray = encrypt(data, true)

    fun encryptHex(data: Any): String = encryptHex(data, true)

    fun encryptBase64(data: Any): String = encryptBase64(data, true)

}

package io.legado.app.help.crypto

import cn.hutool.crypto.symmetric.SymmetricCrypto as HutoolSymmetricCrypto
import io.legado.app.utils.EncoderUtils
import java.io.InputStream
import java.nio.charset.Charset
import javax.crypto.spec.SecretKeySpec

class SymmetricCryptoAndroid(
    algorithm: String,
    key: ByteArray?,
) : HutoolSymmetricCrypto(
    // Key algorithm excludes mode/padding for strict JCE providers.
    algorithm.normalizePkcs7Padding(),
    key?.let { SecretKeySpec(it, algorithm.substringBefore('/')) }
), SymmetricCrypto {

    override fun setIv(iv: ByteArray): SymmetricCryptoAndroid = apply { super<HutoolSymmetricCrypto>.setIv(iv) }
    override fun encrypt(data: ByteArray): ByteArray = super<HutoolSymmetricCrypto>.encrypt(data)
    override fun decrypt(data: ByteArray): ByteArray = super<HutoolSymmetricCrypto>.decrypt(data)
    override fun encrypt(data: String): ByteArray = super<SymmetricCrypto>.encrypt(data)
    override fun encryptHex(data: ByteArray): String = super<SymmetricCrypto>.encryptHex(data)
    override fun encryptHex(data: String): String = super<SymmetricCrypto>.encryptHex(data)
    override fun encryptBase64(data: ByteArray): String = super<SymmetricCrypto>.encryptBase64(data)
    override fun encryptBase64(data: String): String = super<SymmetricCrypto>.encryptBase64(data)
    override fun encryptBase64(data: String, charset: String?): String = super<SymmetricCrypto>.encryptBase64(data, charset)
    override fun decrypt(data: String): ByteArray = super<SymmetricCrypto>.decrypt(data)

    override fun encryptBase64(data: String, charset: Charset?): String = EncoderUtils.base64Encode(encrypt(data, charset))
    override fun encryptBase64(data: InputStream): String = EncoderUtils.base64Encode(encrypt(data))

}

// JCE PKCS5 uses the cipher block size, matching PKCS7 without adding a provider.
internal fun String.normalizePkcs7Padding(): String =
    if (contains("PKCS7Padding", ignoreCase = true)) {
        Regex("(?i)PKCS7Padding").replace(this, "PKCS5Padding")
    } else {
        this
    }

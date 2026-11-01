package io.legado.app.help

import io.legado.app.help.crypto.AsymmetricCrypto
import io.legado.app.help.crypto.AsymmetricCryptoAndroid
import io.legado.app.help.crypto.Sign
import io.legado.app.help.crypto.SignAndroid
import io.legado.app.help.crypto.SymmetricCrypto
import io.legado.app.help.crypto.SymmetricCryptoAndroid

object JsCryptoProviderJvm : JsCryptoProvider {
    override fun digestBytes(algorithm: String, data: ByteArray): ByteArray = java.security.MessageDigest.getInstance(algorithm).digest(data)
    override fun hmacBytes(algorithm: String, key: ByteArray, data: ByteArray): ByteArray = javax.crypto.Mac.getInstance(algorithm).run {
        init(javax.crypto.spec.SecretKeySpec(key, algorithm)); doFinal(data)
    }
    override fun randomBytes(size: Int): ByteArray = ByteArray(size).also { java.security.SecureRandom().nextBytes(it) }

    override fun createSymmetricCrypto(
        transformation: String,
        key: ByteArray?,
        iv: ByteArray?
    ): SymmetricCrypto {
        val crypto = SymmetricCryptoAndroid(transformation, key)
        if (iv != null && iv.isNotEmpty()) {
            if (transformation.split('/').getOrNull(1).equals("GCM", ignoreCase = true)) {
                crypto.setParams(javax.crypto.spec.GCMParameterSpec(128, iv))
            } else crypto.setIv(iv)
        }
        return crypto
    }

    override fun createAsymmetricCrypto(
        transformation: String
    ): AsymmetricCrypto {
        return AsymmetricCryptoAndroid(transformation)
    }

    override fun createSign(
        algorithm: String
    ): Sign {
        return SignAndroid(algorithm)
    }
}

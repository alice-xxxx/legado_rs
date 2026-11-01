package io.legado.app.help

import io.legado.app.help.crypto.AsymmetricCrypto
import io.legado.app.help.crypto.NativeAsymmetricCrypto
import io.legado.app.help.crypto.NativeDigestOps
import io.legado.app.help.crypto.NativeHmacOps
import io.legado.app.help.crypto.NativeSign
import io.legado.app.help.crypto.NativeSymmetricCrypto
import io.legado.app.help.crypto.Sign
import io.legado.app.help.crypto.SymmetricCrypto

object JsCryptoProviderNative : JsCryptoProvider {
    override fun digestBytes(algorithm: String, data: ByteArray): ByteArray = NativeDigestOps.digest(algorithm, data)
    override fun hmacBytes(algorithm: String, key: ByteArray, data: ByteArray): ByteArray = NativeHmacOps.hmac(algorithm, key, data)
    override fun randomBytes(size: Int): ByteArray = io.legado.app.help.crypto.NativeAesOps.randomKey(size)

    override fun createSymmetricCrypto(
        transformation: String,
        key: ByteArray?,
        iv: ByteArray?
    ): SymmetricCrypto {
        val crypto = NativeSymmetricCrypto(transformation, key)
        return if (iv != null && iv.isNotEmpty()) crypto.setIv(iv) else crypto
    }

    override fun createAsymmetricCrypto(
        transformation: String
    ): AsymmetricCrypto {
        return NativeAsymmetricCrypto(transformation)
    }

    override fun createSign(
        algorithm: String
    ): Sign {
        return NativeSign(algorithm)
    }
}

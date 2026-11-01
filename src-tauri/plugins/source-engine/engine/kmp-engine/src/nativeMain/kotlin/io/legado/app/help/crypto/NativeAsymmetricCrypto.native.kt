// 源平台来源：data/src/nativeMain/kotlin/io/legado/app/help/crypto/NativeAsymmetricCrypto.native.kt。此 actual 已复制进提取模块，构建不读取源平台目录。
package io.legado.app.help.crypto

/** Native byte operations; shared interfaces own text, stream and output conversions. */
class NativeAsymmetricCrypto(
    private val algorithm: String,
) : AsymmetricCrypto {

    private var privateKeyBytes: ByteArray? = null
    private var publicKeyBytes: ByteArray? = null

    override fun setPrivateKey(key: ByteArray): AsymmetricCrypto {
        privateKeyBytes = key
        return this
    }

    override fun setPublicKey(key: ByteArray): AsymmetricCrypto {
        publicKeyBytes = key
        return this
    }

    override fun decryptBytes(data: ByteArray, usePublicKey: Boolean?): ByteArray =
        NativeAsymmetricCryptoOps.decrypt(algorithm, usePublicKey == true, privateKeyBytes, publicKeyBytes, data)

    override fun encryptBytes(data: ByteArray, usePublicKey: Boolean?): ByteArray =
        NativeAsymmetricCryptoOps.encrypt(algorithm, usePublicKey == true, privateKeyBytes, publicKeyBytes, data)

}

/**
 * native 端非对称加解密真实实现下沉点 (expect object)。
 *
 * 两端 actual 均为 mbedTLS 主实现 (MbedTlsRsa: v1.5/OAEP + 私钥加密/公钥解密反向 + hutool 同款分块),
 * 任意异常回落既有实现: iOS Security.framework (仅公钥加密/私钥解密单向), 鸿蒙 cryptoFramework napi。
 *
 * usePublicKey: true→用公钥, false/null→用私钥 (对齐 jvmAndAndroid KeyType)。
 * privateKey/publicKey: DER 字节 (私钥 PKCS#8, 公钥 X.509); mbedTLS 主实现另兼容 PKCS#1/PEM。
 */
expect object NativeAsymmetricCryptoOps {
    fun encrypt(
        algorithm: String,
        usePublicKey: Boolean,
        privateKey: ByteArray?,
        publicKey: ByteArray?,
        data: ByteArray
    ): ByteArray

    fun decrypt(
        algorithm: String,
        usePublicKey: Boolean,
        privateKey: ByteArray?,
        publicKey: ByteArray?,
        data: ByteArray
    ): ByteArray
}

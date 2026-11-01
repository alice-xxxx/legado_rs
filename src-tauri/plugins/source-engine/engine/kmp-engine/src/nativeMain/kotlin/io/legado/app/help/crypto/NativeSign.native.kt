// 源平台来源：data/src/nativeMain/kotlin/io/legado/app/help/crypto/NativeSign.native.kt。此 actual 已复制进提取模块，构建不读取源平台目录。
package io.legado.app.help.crypto

/** Native signing operations; shared interfaces own text and output conversions. */
class NativeSign(
    private val algorithm: String,
) : Sign {

    private var privateKeyBytes: ByteArray? = null
    private var publicKeyBytes: ByteArray? = null

    override fun setPrivateKey(key: ByteArray): Sign {
        privateKeyBytes = key
        return this
    }

    override fun setPublicKey(key: ByteArray): Sign {
        publicKeyBytes = key
        return this
    }

    override fun sign(data: ByteArray): ByteArray = NativeSignOps.sign(algorithm, privateKeyBytes, data)
    override fun verify(data: ByteArray, sign: ByteArray): Boolean =
        NativeSignOps.verify(algorithm, publicKeyBytes, data, sign)

}

/**
 * native 端签名/验签真实实现下沉点 (expect object)。
 *
 * 两端 actual 均为 mbedTLS 主实现 (MbedTlsSign: RSA v1.5 含 MD5withRSA、PSS、NONEwithRSA),
 * 任意异常回落既有实现 (iOS Security.framework / 鸿蒙 cryptoFramework napi);
 * ECDSA 在 mbedTLS 裁剪外 (无 ECP), 点名抛异常后由回落承接。
 *
 * sign 用私钥 (PKCS#8 DER), verify 用公钥 (X.509 DER); mbedTLS 主实现另兼容 PKCS#1/PEM。
 */
expect object NativeSignOps {
    fun sign(algorithm: String, privateKey: ByteArray?, data: ByteArray): ByteArray

    fun verify(
        algorithm: String,
        publicKey: ByteArray?,
        data: ByteArray,
        signature: ByteArray
    ): Boolean
}

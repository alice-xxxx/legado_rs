package io.legado.app.help

import io.legado.app.help.crypto.AsymmetricCrypto
import io.legado.app.help.crypto.Sign
import io.legado.app.help.crypto.SymmetricCrypto
import io.legado.app.utils.Base64Lenient
import io.legado.app.utils.MD5Utils
import io.legado.app.utils.toHexLower
import kotlin.concurrent.Volatile

/** 平台提供字节算法与加密工厂；文本编码和摘要输出格式只在公共层维护。 */
interface JsCryptoProvider {
    fun md5Encode(str: String): String = MD5Utils.md5Encode(str)
    fun md5Encode16(str: String): String = MD5Utils.md5Encode16(str)

    fun digestHex(data: String, algorithm: String): String =
        digestBytes(algorithm, data.encodeToByteArray()).toHexLower()

    fun digestBase64Str(data: String, algorithm: String): String =
        Base64Lenient.encodeToString(digestBytes(algorithm, data.encodeToByteArray()))

    @Suppress("FunctionName")
    fun HMacHex(data: String, algorithm: String, key: String): String =
        hmacBytes(algorithm, key.encodeToByteArray(), data.encodeToByteArray()).toHexLower()

    @Suppress("FunctionName")
    fun HMacBase64(data: String, algorithm: String, key: String): String =
        Base64Lenient.encodeToString(hmacBytes(algorithm, key.encodeToByteArray(), data.encodeToByteArray()))

    fun digestBytes(algorithm: String, data: ByteArray): ByteArray
    fun hmacBytes(algorithm: String, key: ByteArray, data: ByteArray): ByteArray
    fun randomBytes(size: Int): ByteArray

    /**
     * 创建对称加解密器。key 为 null 时使用随机密钥, iv 非空时设置初始向量。
     * 不支持的算法/模式由平台实现构造时抛错。
     */
    fun createSymmetricCrypto(
        transformation: String,
        key: ByteArray?,
        iv: ByteArray?
    ): SymmetricCrypto

    /**
     * 创建非对称加解密器
     */
    fun createAsymmetricCrypto(
        transformation: String
    ): AsymmetricCrypto

    /**
     * 创建签名器
     */
    fun createSign(
        algorithm: String
    ): Sign
}

object JsCryptoProviders {
    @Volatile
    private var impl: JsCryptoProvider? = null

    fun register(impl: JsCryptoProvider) {
        this.impl = impl
    }

    fun get(): JsCryptoProvider = impl ?: error("JsCryptoProvider 未注册")
}

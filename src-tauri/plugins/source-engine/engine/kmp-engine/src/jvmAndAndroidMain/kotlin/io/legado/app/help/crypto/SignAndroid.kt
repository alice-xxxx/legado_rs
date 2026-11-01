package io.legado.app.help.crypto

import cn.hutool.crypto.KeyUtil

@Suppress("unused")
class SignAndroid(algorithm: String): cn.hutool.crypto.asymmetric.Sign(algorithm), Sign {

    override fun setPrivateKey(key: ByteArray): SignAndroid {
        setPrivateKey(KeyUtil.generatePrivateKey(algorithm, key))
        return this
    }

    override fun setPrivateKey(key: String): SignAndroid = setPrivateKey(key.encodeToByteArray())

    override fun setPublicKey(key: ByteArray): SignAndroid {
        setPublicKey(KeyUtil.generatePublicKey(algorithm, key))
        return this
    }

    override fun setPublicKey(key: String): SignAndroid = setPublicKey(key.encodeToByteArray())

    override fun sign(data: String): ByteArray = super<Sign>.sign(data)
    override fun signHex(data: ByteArray): String = super<Sign>.signHex(data)
    override fun signHex(data: String): String = super<Sign>.signHex(data)

}

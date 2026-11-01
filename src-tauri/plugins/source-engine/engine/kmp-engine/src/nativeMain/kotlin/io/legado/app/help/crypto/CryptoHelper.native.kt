package io.legado.app.help.crypto

internal actual fun aesEcbPkcs5(key: ByteArray, data: ByteArray, encrypt: Boolean): ByteArray =
    if (encrypt) NativeAesOps.encryptEcbPkcs7(key, data) else NativeAesOps.decryptEcbPkcs7(key, data)

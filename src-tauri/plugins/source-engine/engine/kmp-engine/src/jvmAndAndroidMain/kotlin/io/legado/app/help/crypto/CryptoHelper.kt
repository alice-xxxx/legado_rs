package io.legado.app.help.crypto

import javax.crypto.Cipher
import javax.crypto.spec.SecretKeySpec

internal actual fun aesEcbPkcs5(key: ByteArray, data: ByteArray, encrypt: Boolean): ByteArray =
    Cipher.getInstance("AES/ECB/PKCS5Padding").run {
        init(if (encrypt) Cipher.ENCRYPT_MODE else Cipher.DECRYPT_MODE, SecretKeySpec(key, "AES"))
        doFinal(data)
    }

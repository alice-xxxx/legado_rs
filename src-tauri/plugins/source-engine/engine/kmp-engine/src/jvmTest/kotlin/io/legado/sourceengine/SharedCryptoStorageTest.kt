package io.legado.sourceengine

import io.legado.app.help.crypto.CryptoHelper
import java.util.Base64
import javax.crypto.Cipher
import javax.crypto.spec.SecretKeySpec
import kotlin.test.*

class SharedCryptoStorageTest {
    @Test fun loginInfoKeepsTheExistingCiphertextWithoutRuntimeRegistration() {
        for (size in listOf(16, 24, 32)) {
            val key = ByteArray(size) { it.toByte() }
            for (text in listOf("", "账号😀\u0000", "\uD800tail", "chapter".repeat(2000))) {
                val cipher = Cipher.getInstance("AES/ECB/PKCS5Padding")
                cipher.init(Cipher.ENCRYPT_MODE, SecretKeySpec(key, "AES"))
                val expected = Base64.getEncoder().encodeToString(cipher.doFinal(text.toByteArray()))
                assertEquals(expected, CryptoHelper.encryptAesEcbPkcs5Base64(key, text))
                val decoded = text.toByteArray().decodeToString()
                assertEquals(decoded, CryptoHelper.decryptAesEcbPkcs5Base64(key, expected))
                assertEquals(decoded, CryptoHelper.decryptAesEcbPkcs5Base64(key, "\n" + expected.chunked(5).joinToString(" ")))
            }
        }
    }

    @Test fun invalidCipherInputsDoNotRetainState() {
        assertFails { CryptoHelper.encryptAesEcbPkcs5Base64(ByteArray(3), "value") }
        assertFails { CryptoHelper.decryptAesEcbPkcs5Base64(ByteArray(16), "AA==") }
        val encrypted = CryptoHelper.encryptAesEcbPkcs5Base64(ByteArray(16), "value")
        assertEquals("value", CryptoHelper.decryptAesEcbPkcs5Base64(ByteArray(16), encrypted))
    }
}

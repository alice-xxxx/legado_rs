package io.legado.sourceengine

import cn.hutool.crypto.SecureUtil
import io.legado.app.help.crypto.*
import io.legado.app.model.analyzeRule.AnalyzeRuleCore
import io.legado.app.model.script.NATIVE_CRYPTO_BOOTSTRAP
import io.legado.app.utils.toHexLower
import io.legado.sourceengine.host.HostRuntime
import java.io.ByteArrayInputStream
import java.security.KeyPairGenerator
import java.security.Signature
import java.util.Base64
import javax.crypto.Cipher
import javax.crypto.spec.SecretKeySpec
import kotlin.test.*

class SharedCryptoObjectsTest {
    init { HostRuntime.install { error("Unexpected image operation") } }

    @Test fun symmetricTextAndEncodingsMatchJce() {
        val key = ByteArray(16) { it.toByte() }
        val crypto: SymmetricCrypto = SymmetricCryptoAndroid("AES/ECB/PKCS7Padding", key)
        for (text in listOf("", "中文😀\u0000", "\uD800tail", "chapter".repeat(100))) {
            val cipher = Cipher.getInstance("AES/ECB/PKCS5Padding")
            cipher.init(Cipher.ENCRYPT_MODE, SecretKeySpec(key, "AES"))
            val expected = cipher.doFinal(text.toByteArray())
            assertContentEquals(expected, crypto.encrypt(text))
            assertEquals(expected.toHexLower(), crypto.encryptHex(text))
            assertEquals(Base64.getEncoder().encodeToString(expected), crypto.encryptBase64(text))
            assertContentEquals(text.toByteArray(), crypto.decrypt(expected))
            assertContentEquals(text.toByteArray(), crypto.decrypt(expected.toHexLower()))
            assertContentEquals(text.toByteArray(), crypto.decrypt("\n" + Base64.getEncoder().encodeToString(expected).chunked(5).joinToString(" ")))
        }
        for (charset in listOf("UTF-8", "UTF-16LE", "GBK", "windows-1252")) {
            val expected = crypto.encrypt("中文ABC".toByteArray(charset(charset)))
            assertEquals(Base64.getEncoder().encodeToString(expected), crypto.encryptBase64("中文ABC", charset))
        }
        assertFails { crypto.decrypt("AA==") }
        assertContentEquals("next".toByteArray(), crypto.decrypt(crypto.encrypt("next")))
        val cbc: SymmetricCrypto = SymmetricCryptoAndroid("AES/CBC/PKCS5Padding", key)
        val iv = ByteArray(16) { (16 - it).toByte() }
        assertSame(cbc, cbc.setIv(iv))
        val cipher = Cipher.getInstance("AES/CBC/PKCS5Padding")
        cipher.init(Cipher.ENCRYPT_MODE, SecretKeySpec(key, "AES"), javax.crypto.spec.IvParameterSpec(iv))
        assertContentEquals(cipher.doFinal("chapter".toByteArray()), cbc.encrypt("chapter"))
    }

    @Test fun cipherDecodingKeepsHutoolHexPrecedenceAndOddDigits() {
        for (data in listOf("", "0", "abc", "ABCDEF", "00ff", "AA==", "AA-_", " Zm 9v\n", "Z!m@9#v$")) {
            assertContentEquals(SecureUtil.decode(data), decodeCipherText(data), data)
        }
    }

    private val keys by lazy { KeyPairGenerator.getInstance("RSA").apply { initialize(1024) }.generateKeyPair() }

    @Test fun asymmetricBinaryStreamsAndKeyDirectionsMatchJce() {
        val crypto: AsymmetricCrypto = AsymmetricCryptoAndroid("RSA/ECB/NoPadding")
            .setPrivateKey(keys.private.encoded).setPublicKey(keys.public.encoded)
        val data = "中文😀".toByteArray()
        for (usePublic in listOf(true, false, null)) {
            val cipher = Cipher.getInstance("RSA/ECB/NoPadding")
            cipher.init(Cipher.ENCRYPT_MODE, if (usePublic == true) keys.public else keys.private)
            val expected = cipher.doFinal(data)
            assertContentEquals(expected, crypto.encrypt(data, usePublic))
            assertContentEquals(expected, crypto.encrypt("中文😀", usePublic))
            assertEquals(expected.toHexLower(), crypto.encryptHex(data, usePublic))
            assertEquals(Base64.getEncoder().encodeToString(expected), crypto.encryptBase64(data, usePublic))
            val input = TrackingStream(data)
            assertContentEquals(expected, crypto.encrypt(input, usePublic))
            assertTrue(input.closed)
            val padded = ByteArray(128 - data.size) + data
            assertContentEquals(padded, crypto.decrypt(expected.toHexLower(), usePublic != true))
            assertContentEquals(padded, crypto.decrypt(Base64.getEncoder().encodeToString(expected), usePublic != true))
            val encryptedInput = TrackingStream(expected)
            assertContentEquals(padded, crypto.decrypt(encryptedInput, usePublic != true))
            assertTrue(encryptedInput.closed)
        }
        assertFailsWith<IllegalArgumentException> { crypto.encrypt(42, true) }
        val failing = object : ByteArrayInputStream(byteArrayOf()) {
            var closed = false
            override fun read(b: ByteArray, off: Int, len: Int): Int = error("read failure")
            override fun close() { closed = true }
        }
        assertFails { crypto.encrypt(failing, true) }
        assertTrue(failing.closed)
        assertEquals(128, crypto.encrypt(data, true).size)
    }

    @Test fun signatureTextAndHexShareTheBinaryOperation() {
        val sign: Sign = SignAndroid("SHA256withRSA").setPrivateKey(keys.private.encoded).setPublicKey(keys.public.encoded)
        for (text in listOf("", "中文😀\u0000", "\uD800tail")) {
            val jdk = Signature.getInstance("SHA256withRSA")
            jdk.initSign(keys.private)
            jdk.update(text.toByteArray())
            val expected = jdk.sign()
            assertContentEquals(expected, sign.sign(text))
            assertEquals(expected.toHexLower(), sign.signHex(text))
            assertEquals(expected.toHexLower(), sign.signHex(text.toByteArray()))
            assertTrue(sign.verify(text, expected))
            assertFalse(sign.verify(text + "bad", expected))
        }
    }

    @Test fun scriptSingleArgumentOverloadsAndNativeWrapperDefaultsRemainCallable() {
        val rule = AnalyzeRuleCore()
        assertEquals("128", rule.evalJS("java.createAsymmetricCrypto('RSA/ECB/NoPadding').encrypt('a').length").toString())
        assertEquals("256", rule.evalJS("java.createAsymmetricCrypto('RSA/ECB/NoPadding').encryptHex('a').length").toString())
        assertEquals("true", rule.evalJS(NATIVE_CRYPTO_BOOTSTRAP + """
            var calls = [];
            function __nativeDispatch(h, m, a) { calls.push([h, m, a]); return a; }
            var a = __createAsymCryptoObj(1), s = __createSignObj(2), c = __createCryptoObj(3);
            var same = a.setPrivateKey([0,255]) === a && s.setPublicKey([1,128]) === s;
            var defaults = ['encrypt','decrypt','decryptStr','encryptHex','encryptBase64'].every(function(m) {
                return a[m]('x')[1] === true && a[m]('x',undefined)[1] === true &&
                    a[m]('x',false)[1] === false && a[m]('x',null)[1] === null;
            });
            same && defaults && calls[0][2][0][1] === 255 && calls[1][2][0][1] === 128 &&
                c.encryptBase64('x','GBK')[1] === 'GBK';
        """).toString())
    }

    private class TrackingStream(data: ByteArray) : ByteArrayInputStream(data) {
        var closed = false
        override fun close() { closed = true; super.close() }
    }
}

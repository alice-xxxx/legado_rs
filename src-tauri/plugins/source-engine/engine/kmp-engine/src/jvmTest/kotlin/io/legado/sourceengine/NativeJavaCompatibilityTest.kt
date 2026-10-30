package io.legado.sourceengine

import io.legado.app.model.script.*
import io.legado.app.utils.KS_JSON
import io.legado.sourceengine.host.HostRuntime
import kotlinx.serialization.json.*
import kotlin.test.*

/** 在真实 QuickJS 中验证 Native 共用的包入口、代理和分派逻辑；不替代 iOS C 桥接验收。 */
class NativeJavaCompatibilityTest {
    class Bridge {
        private val objects = mutableMapOf<Int, Any>()
        fun dispatch(handle: Int, method: Int, json: String): String {
            fun decode(value: JsonElement): Any? = when (value) {
                JsonNull -> null
                is JsonArray -> value.map { decode(it) }
                is JsonObject -> if ("ref" in value) objects.getValue(value.getValue("ref").jsonPrimitive.int) else value.mapValues { decode(it.value) }
                is JsonPrimitive -> if (value.isString) value.content else value.booleanOrNull ?: value.double
            }
            fun encode(value: Any?): JsonElement = when {
                value == null -> JsonNull
                JavaClassCompat.supports(value) -> {
                    val handle = objects.size + 1
                    objects[handle] = value
                    buildJsonObject { put("ref", handle); put("collection", JavaClassCompat.isCollection(value)) }
                }
                value is String -> JsonPrimitive(value)
                value is Boolean -> JsonPrimitive(value)
                value is Number -> JsonPrimitive(value)
                value is ByteArray -> JsonArray(value.map { JsonPrimitive(it.toInt()) })
                value is Map<*, *> -> JsonObject(value.entries.associate { it.key.toString() to encode(it.value) })
                value is List<*> -> JsonArray(value.map { encode(it) })
                else -> error("Unexpected fixture return type")
            }
            val args = decode(KS_JSON.parseToJsonElement(json)) as List<Any?>
            val value = when (method) {
                3400 -> JavaClassCompat.loadClass(args[0] as String)
                3401 -> JavaClassCompat.construct((args[0] as Number).toInt(), args[1] as List<Any?>)
                3402 -> JavaClassCompat.callStatic((args[0] as Number).toInt(), args[1] as String, args[2] as List<Any?>)
                3404 -> JavaClassCompat.field((args[0] as Number).toInt(), args[1] as String)
                else -> JavaClassCompat.call(objects.getValue(handle), args[0] as String, args[1] as List<Any?>)
            }
            return encode(value).toString()
        }
    }

    init { HostRuntime.install { error("Image operation outside fixture") } }

    private fun eval(script: String): String {
        val engine = JsEngines.current
        val scope = engine.getRuntimeScope(buildScriptBindings { it["bridge"] = Bridge() })
        val transport = """
            function __nativeDispatch(handle, method, args) {
                function pack(v) {
                    if (v && typeof v.__javaCompatHandle__ === 'number') return {ref:v.__javaCompatHandle__};
                    return Array.isArray(v) ? v.map(pack) : v;
                }
                var value = JSON.parse(bridge.dispatch(handle, method, JSON.stringify(pack(args))));
                return value && value.ref ? __createJavaCompatObj(value.ref, value.collection) : value;
            }
        """
        return try { engine.eval(transport + NATIVE_JAVA_BOOTSTRAP + "\n" + script, scope, null).toString() }
        finally { scope.close() }
    }

    @Test fun nativePackagesConstructAndMutateCards() {
        assertEquals("我的小说:1:/book", eval("""
            var cards = new org.jsoup.select.Elements();
            var card = new org.jsoup.nodes.Element('div');
            card.appendChild(new org.jsoup.nodes.Element('a').attr('href','/book').text('我的小说'));
            cards.add(card);
            cards[0].text() + ':' + Array.from(cards).length + ':' + cards.first().select('a').attr('href');
        """))
    }

    @Test fun importedClassesAndHtmlSerializationWork() {
        assertEquals("<p>Value</p>", eval("""
            var imports = new JavaImporter(Packages.org.jsoup);
            with(imports) { JSON.parse(JSON.stringify(Jsoup.parse('<p>Value</p>').selectFirst('p'))); }
        """))
        assertEquals("value", eval("String(new Packages.java.lang.String('value'))"))
        assertEquals("true", eval("!__classExists('java.lang.Runtime') && __classExists('org.jsoup.Jsoup')"))
        assertEquals("POST", eval("String(org.jsoup.Connection.Method.POST)"))
        assertEquals("ready",eval("java.lang.Thread.sleep(0);'ready';"))
    }

    @Test fun binaryDigestsAndBase64MatchJava() {
        val bytes = byteArrayOf(0, -1, -128, 65)
        val expected = java.util.Base64.getEncoder().encodeToString(java.security.MessageDigest.getInstance("SHA-256").digest(bytes))
        assertEquals(expected, eval("""
            var digest = java.security.MessageDigest.getInstance('SHA-256');
            digest.update([0,-1]); digest.update([-128,65]);
            java.util.Base64.getEncoder().encodeToString(digest.digest());
        """))
        assertEquals("value", eval("String(new java.lang.String(android.util.Base64.decode('dmFsdWU=',android.util.Base64.DEFAULT),'UTF-8'))"))
        assertEquals("true", eval("try {java.util.Base64.getDecoder().decode('!!');false;} catch(e) {true;}"))
    }

    @Test fun aesCipherAndHmacUseBinaryKeysAndInitialization() {
        val key = javax.crypto.spec.SecretKeySpec(ByteArray(16), "AES")
        val cipher = javax.crypto.Cipher.getInstance("AES/CBC/PKCS5Padding")
        cipher.init(javax.crypto.Cipher.ENCRYPT_MODE, key, javax.crypto.spec.IvParameterSpec(ByteArray(16)))
        val ciphertext = java.util.Base64.getEncoder().encodeToString(cipher.doFinal("我的系统".toByteArray()))
        assertEquals("我的系统", eval("""
            var b=java.util.Base64.getDecoder(), C=javax.crypto.Cipher;
            var c=C.getInstance('AES/CBC/PKCS5Padding');
            var key=new javax.crypto.spec.SecretKeySpec(b.decode('AAAAAAAAAAAAAAAAAAAAAA=='),'AES');
            c.init(C.DECRYPT_MODE,key,new javax.crypto.spec.IvParameterSpec(key.getEncoded()));
            String(new java.lang.String(c.doFinal(b.decode('$ciphertext')),'UTF-8'));
        """))
        val mac=javax.crypto.Mac.getInstance("HmacSHA256")
        mac.init(javax.crypto.spec.SecretKeySpec(byteArrayOf(0,-1),"HmacSHA256"))
        val expected=java.util.Base64.getEncoder().encodeToString(mac.doFinal(byteArrayOf(-128,65)))
        assertEquals(expected,eval("var m=javax.crypto.Mac.getInstance('HmacSHA256');m.init(new javax.crypto.spec.SecretKeySpec([0,-1],'HmacSHA256'));java.util.Base64.getEncoder().encodeToString(m.doFinal([-128,65]));"))
    }

    @Test fun gcmCipherAuthenticatesAndSecureRandomWritesIntoTheGivenArray() {
        val cipher=javax.crypto.Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(javax.crypto.Cipher.ENCRYPT_MODE,javax.crypto.spec.SecretKeySpec(ByteArray(16),"AES"),javax.crypto.spec.GCMParameterSpec(128,ByteArray(12)))
        val expected=java.util.Base64.getEncoder().encodeToString(cipher.doFinal(ByteArray(16)))
        assertEquals(expected,eval("""
            var c=javax.crypto.Cipher.getInstance('AES/GCM/NoPadding');
            var key=new javax.crypto.spec.SecretKeySpec(Array(16).fill(0),'AES');
            var iv=new javax.crypto.spec.GCMParameterSpec(128,Array(12).fill(0));
            c.init(javax.crypto.Cipher.ENCRYPT_MODE,key,iv);
            java.util.Base64.getEncoder().encodeToString(c.doFinal(Array(16).fill(0)));
        """))
        assertEquals("true",eval("""
            var c=javax.crypto.Cipher.getInstance('AES/GCM/NoPadding');
            c.init(2,new javax.crypto.spec.SecretKeySpec(Array(16).fill(0),'AES'),new javax.crypto.spec.GCMParameterSpec(128,Array(12).fill(0)));
            try { c.doFinal(Array(32).fill(0));false; } catch(e) { true; }
        """))
        assertEquals("true",eval("var a=Array(16).fill(null);new java.security.SecureRandom().nextBytes(a);a.every(x=>Number.isInteger(x)&&x>=-128&&x<=127);"))
    }

    @Test fun javaCollectionsKeepTheirViewsAndIterators() {
        assertEquals("Value:2:key,other",eval("""
            var map=new java.util.LinkedHashMap();map.put('key','Value');
            var keys=map.keySet();map.put('other','Second');
            var list=new java.util.ArrayList(keys.toArray());
            map.get('key')+':'+keys.size()+':'+Array.from(list).join(',');
        """))
        assertEquals("{\"X-Test\":\"verified\"}",eval("var m=new java.util.HashMap();m.put('X-Test','verified');JSON.stringify(m);"))
        assertEquals("b:1",eval("var a=new java.util.ArrayList(['a','b']);var i=a.iterator();i.next();i.remove();i.next()+':'+a.size();"))
        assertEquals("New:0",eval("var m=new java.util.HashMap();m.put('a','Old');var es=m.entrySet();es.iterator().next().setValue('New');var text=m.get('a');es.clear();text+':'+m.size();"))
    }
}

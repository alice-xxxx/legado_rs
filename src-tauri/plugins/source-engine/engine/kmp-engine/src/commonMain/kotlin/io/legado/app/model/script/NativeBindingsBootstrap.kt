package io.legado.app.model.script

/** Inject generated methods inside their owning factory, after the compatibility wrappers. */
internal fun nativeBindingFactories(methodTables: Map<String, String>): String {
    var code = EXTENSION_FACTORIES + "\n" + PROPERTY_FACTORIES
    for ((factory, table) in methodTables) {
        code = code.replace("    // @@methods:$factory@@", table.trim('\n'))
    }
    return code
}

private val EXTENSION_FACTORIES = """
// ============ JsExtensions 桥接工厂 (handle → JS 对象) ============

function __createImageOpsObj(handle) {
    // ImageRef is an opaque QuickJS object. Normalized PNG Uint8Arrays stay in this
    // scope-local WeakMap and never become Rust/global handles or WebView resources.
    var refs = new WeakMap();
    function wrap(bytes) {
        var ref = {};
        refs.set(ref, bytes);
        return ref;
    }
    function unwrap(ref) {
        if (!ref || typeof ref !== 'object' || !refs.has(ref)) {
            throw new TypeError('image: argument is not an image.* result');
        }
        return refs.get(ref);
    }
    var obj = {};
    obj.decode = function(value) {
        return wrap(__nativeDispatch(handle, 3301, [value]));
    };
    obj.encode = function(img, format, quality) {
        return __nativeDispatch(handle, 3302, [unwrap(img), format, quality]);
    };
    obj.split = function(img, rows, cols) {
        return __nativeDispatch(handle, 3303, [unwrap(img), rows, cols]).map(wrap);
    };
    obj.stitch = function(imgs, direction) {
        if (!Array.isArray(imgs)) throw new TypeError('image.stitch expects an array');
        return wrap(__nativeDispatch(handle, 3304, [imgs.map(unwrap), direction]));
    };
    obj.crop = function(img, x, y, w, h) {
        return wrap(__nativeDispatch(handle, 3305, [unwrap(img), x, y, w, h]));
    };
    obj.rotate = function(img, deg) {
        return wrap(__nativeDispatch(handle, 3306, [unwrap(img), deg]));
    };
    obj.flip = function(img, direction) {
        return wrap(__nativeDispatch(handle, 3307, [unwrap(img), direction]));
    };
    obj.size = function(img) {
        return __nativeDispatch(handle, 3308, [unwrap(img)]);
    };
    return obj;
}

function __createJavaObj(handle) {
    var obj = {};
    // JsCryptoProviders 摘要/HMAC 面 (100-199)
    obj.md5Encode = function(str) { return __nativeDispatch(handle, 101, [str]); };
    obj.md5Encode16 = function(str) { return __nativeDispatch(handle, 102, [str]); };
    obj.digestHex = function(data, algorithm) { return __nativeDispatch(handle, 103, [data, algorithm]); };
    obj.digestBase64Str = function(data, algorithm) { return __nativeDispatch(handle, 104, [data, algorithm]); };
    obj.HMacHex = function(data, algorithm, key) { return __nativeDispatch(handle, 105, [data, algorithm, key]); };
    obj.HMacBase64 = function(data, algorithm, key) { return __nativeDispatch(handle, 106, [data, algorithm, key]); };
    // 工厂方法 (200-299) → 返回 handle, 用 __createCryptoObj 包装
    obj.createSymmetricCrypto = function(transformation, key, iv) {
        var cryptoHandle = __nativeDispatch(handle, 201, [transformation, key, iv]);
        return __createCryptoObj(cryptoHandle);
    };
    obj.desEncodeToBase64String = function(data, key, transformation, iv) {
        return obj.createSymmetricCrypto(transformation, key, iv || null).encryptBase64(data);
    };
    obj.aesEncodeToBase64String = obj.desEncodeToBase64String;
    obj.createAsymmetricCrypto = function(transformation) {
        var cryptoHandle = __nativeDispatch(handle, 202, [transformation]);
        return __createAsymCryptoObj(cryptoHandle);
    };
    obj.createSign = function(algorithm) {
        var signHandle = __nativeDispatch(handle, 203, [algorithm]);
        return __createSignObj(signHandle);
    };
    // JsExtensionsCommon overloads retain argument-count dispatch.
    obj.strToBytes = function(str, charset) {
        return arguments.length >= 2 ? __nativeDispatch(handle, 13, [str, charset]) : __nativeDispatch(handle, 3, [str]);
    };
    obj.bytesToStr = function(bytes, charset) {
        return arguments.length >= 2 ? __nativeDispatch(handle, 14, [bytes, charset]) : __nativeDispatch(handle, 4, bytes);
    };
    obj.base64Decode = function(str, second) {
        return arguments.length >= 2 ? __nativeDispatch(handle, 15, [str, second]) : __nativeDispatch(handle, 2, [str]);
    };
    obj.base64Encode = function(str, flags) {
        return arguments.length >= 2 ? __nativeDispatch(handle, 17, [str, flags]) : __nativeDispatch(handle, 1, [str]);
    };
    obj.encodeURI = function(str, enc) {
        return arguments.length >= 2 ? __nativeDispatch(handle, 19, [str, enc]) : __nativeDispatch(handle, 7, [str]);
    };
    // ============ 编解码补齐 (13-29) ============
    obj.base64DecodeToByteArray = function(str, flags) { return __nativeDispatch(handle, 16, [str, flags]); };
    obj.toURL = function(url, baseUrl) { return __createJsUrlObj(__nativeDispatch(handle, 22, [url, baseUrl])); };
    // ============ 规则引擎核心补齐 (23-28, 对齐 JVM @JsApi 分派表) ============
    obj.getSource = function() { var h = __nativeDispatch(handle, 23, []); return h ? __createBaseSourceObj(h) : null; };
    obj.put = function(key, value) { return __nativeDispatch(handle, 24, [key, value]); };
    obj.evalJS = function(jsStr) { return __nativeDispatch(handle, 26, [jsStr]); };
    obj.getString = function(rule, content, isUrl) { return __nativeDispatch(handle, 27, [rule, content, isUrl]); };
    obj.getStringList = function(rule, content, isUrl) { return __nativeDispatch(handle, 28, [rule, content, isUrl]); };
    // ============ 网络族 (300-399) ============
    obj.ajax = function(url, timeoutMs) {
        return __nativeDispatch(handle, 301, arguments.length >= 2 ? [url, timeoutMs] : [url]);
    };
    obj.ajaxAll = function(urlList) {
        var hs = __nativeDispatch(handle, 302, [urlList]);
        if (!hs) return [];
        var out = [];
        for (var i = 0; i < hs.length; i++) out.push(__createStrResponseObj(hs[i]));
        return out;
    };
    obj.connect = function(urlStr, header) { return __createStrResponseObj(__nativeDispatch(handle, 303, [urlStr, header])); };
    obj.webView = function(...args) { return __nativeDispatch(handle, 304, args); };
    obj.webViewGetSource = function(...args) { return __nativeDispatch(handle, 305, args); };
    obj.webViewGetOverrideUrl = function(...args) { return __nativeDispatch(handle, 306, args); };
    obj.getCookie = function(tag, key) { return __nativeDispatch(handle, 307, [tag, key]); };
    // java.get 重载: 单参 = 变量读取 (25), 双/三参 = 网络 GET (309)。
    obj.get = function(a, b, timeoutMs) {
        if (arguments.length >= 2) {
            var h = (typeof b === 'string') ? b : JSON.stringify(b);
            var params = arguments.length >= 3 ? [a, h, timeoutMs] : [a, h];
            return __createRespObj(__nativeDispatch(handle, 309, params));
        }
        return __nativeDispatch(handle, 25, [a]);
    };
    obj.head = function(urlStr, headers) {
        var h = (typeof headers === 'string') ? headers : JSON.stringify(headers);
        return __createRespObj(__nativeDispatch(handle, 310, [urlStr, h]));
    };
    obj.post = function(urlStr, body, headers, timeoutMs) {
        var h = (typeof headers === 'string') ? headers : JSON.stringify(headers);
        var params = arguments.length >= 4 ? [urlStr, body, h, timeoutMs] : [urlStr, body, h];
        return __createRespObj(__nativeDispatch(handle, 311, params));
    };
    // ============ UI/杂项 (400-499) ============
    obj.log = function(msg) { __nativeDispatch(handle, 402, [msg]); return msg; };
    obj.logType = function(any) { __nativeDispatch(handle, 403, [any]); };
    obj.openUrl = function(url, mimeType) { __nativeDispatch(handle, 407, [url, mimeType]); };
    obj.deviceID = function() { return __nativeDispatch(handle, 414, []); };
    obj.refreshExplore = function() { __nativeDispatch(handle, 415, []); };
    obj.upLoginData = function(data) { __nativeDispatch(handle, 416, [JSON.stringify(data)]); };
    obj.reLoginView = function(deltaUp) { __nativeDispatch(handle, 417, [deltaUp]); };
    obj.startBrowser = function(url, title) { __nativeDispatch(handle, 408, [url, title]); };
    obj.startBrowserAwait = function(url, title, refetchAfterSuccess) {
        return __createStrResponseObj(__nativeDispatch(handle, 409, [url, title, refetchAfterSuccess]));
    };
    // ============ 字体族 (500-599) ============
    obj.queryTTF = function(data, useCache) { return __createQueryTTFObj(__nativeDispatch(handle, 501, [data, useCache])); };
    obj.queryBase64TTF = function(data) { return __createQueryTTFObj(__nativeDispatch(handle, 502, [data])); };
    obj.replaceFont = function(text, errorQueryTTF, correctQueryTTF, filter) {
        var e = errorQueryTTF ? errorQueryTTF.__h : null;
        var c = correctQueryTTF ? correctQueryTTF.__h : null;
        return __nativeDispatch(handle, 503, [text, e, c, filter === true]);
    };
    // ============ 文件/压缩族 (600-699) ============
    obj.readTxtFile = function(path, charsetName) { return __nativeDispatch(handle, 603, [path, charsetName]); };
    obj.getZipStringContent = function(url, path, charsetName) { return __nativeDispatch(handle, 610, [url, path, charsetName]); };
    obj.getRarStringContent = function(url, path, charsetName) { return __nativeDispatch(handle, 611, [url, path, charsetName]); };
    obj.get7zStringContent = function(url, path, charsetName) { return __nativeDispatch(handle, 612, [url, path, charsetName]); };
    obj.downloadFile = function(a, b) { return __nativeDispatch(handle, 616, [a, b]); };
    obj.cacheFile = function(urlStr, saveTime) { return __nativeDispatch(handle, 617, [urlStr, saveTime]); };
    // @@methods:__createJavaObj@@
    return obj;
}

$NATIVE_CRYPTO_BOOTSTRAP

function __createQueryTTFObj(handle) {
    if (!handle || handle <= 0) return null;
    var obj = {};
    obj.__h = handle; // replaceFont 回传句柄用
    // getGlyfById 等规整函数手写闭包 (生成表注入点在下方 @@methods 标记处, 位于其后,
    // 同名定义以生成为准); 映射表属性: 首次访问时 JSON 反序列化并缓存 (生成器只收集函数)
    obj.getGlyfById = function(glyfId) { return __nativeDispatch(handle, 1301, [glyfId]); };
    obj.getGlyfIdByUnicode = function(unicode) { return __nativeDispatch(handle, 1302, [unicode]); };
    obj.getGlyfByUnicode = function(unicode) { return __nativeDispatch(handle, 1303, [unicode]); };
    obj.getUnicodeByGlyf = function(glyph) { return __nativeDispatch(handle, 1304, [glyph]); };
    obj.isBlankUnicode = function(unicode) { return __nativeDispatch(handle, 1305, [unicode]); };
    // 映射表属性: 首次访问时 JSON 反序列化并缓存
    [["unicodeToGlyph", 1306], ["glyphToUnicode", 1307], ["unicodeToGlyphId", 1308]].forEach(function(m) {
        var cache = null;
        Object.defineProperty(obj, m[0], {
            get: function() {
                if (cache === null) cache = JSON.parse(__nativeDispatch(handle, m[1], []) || "{}");
                return cache;
            }
        });
    });
    // @@methods:__createQueryTTFObj@@
    return obj;
}

function __createStrResponseObj(handle) {
    if (!handle || handle <= 0) return null;
    var obj = {};
    obj.__h = handle;
    // 规整方法 (body/url/code/message/isSuccessful) 由生成表注入 (下方标记处)
    obj.headers = function() { return __nativeDispatch(handle, 1406, []); };
    // @@methods:__createStrResponseObj@@
    return obj;
}

function __createRespObj(handle) {
    if (!handle || handle <= 0) return null;
    var obj = {};
    obj.__h = handle;
    // Connection.Response 方法面: 规整方法由生成表注入 (下方标记处);
    // 手写方法：704 url()、712 method() 及 Connection.Base 的请求头/cookie 重载
    obj.url = function() { return __nativeDispatch(handle, 704, []); };
    obj.method = function() { return __nativeDispatch(handle, 712, []); };
    // @@methods:__createRespObj@@
    // 手动分派全部重载，避免 KSP 把 nullable setter 当成不可生成的重载。
    obj.header = function(name, value) {
        if (arguments.length < 2) return __nativeDispatch(handle, 718, [name]);
        __nativeDispatch(handle, 713, [name, value]);
        return obj;
    };
    obj.addHeader = function(name, value) {
        __nativeDispatch(handle, 714, [name, value]);
        return obj;
    };
    obj.removeHeader = function(name) {
        __nativeDispatch(handle, 715, [name]);
        return obj;
    };
    obj.cookie = function(name, value) {
        if (arguments.length < 2) return __nativeDispatch(handle, 719, [name]);
        __nativeDispatch(handle, 716, [name, value]);
        return obj;
    };
    obj.removeCookie = function(name) {
        __nativeDispatch(handle, 717, [name]);
        return obj;
    };
    return obj;
}

function __createJsUrlObj(handle) {
    if (!handle || handle <= 0) return null;
    var obj = {};
    obj.__h = handle;
    Object.defineProperty(obj, "host", { get: function() { return __nativeDispatch(handle, 1501, []); } });
    Object.defineProperty(obj, "origin", { get: function() { return __nativeDispatch(handle, 1502, []); } });
    Object.defineProperty(obj, "pathname", { get: function() { return __nativeDispatch(handle, 1503, []); } });
    Object.defineProperty(obj, "searchParams", { get: function() {
        var s = __nativeDispatch(handle, 1504, []);
        return (s === null || s === undefined) ? null : JSON.parse(s);
    } });
    // @@methods:__createJsUrlObj@@
    return obj;
}
    """.trimIndent()

private val PROPERTY_FACTORIES = """
// ============ 复杂对象属性桥工厂 (NativeJsPropertyBridge, propId 1700-2199) ============

function __createBaseSourceObj(handle) {
    if (!handle || handle <= 0) return null;
    // BaseSource 实现 JsExtensionsCommon, 继承完整 java 方法面 (ajax/getHeaderMap/put/get 等)
    var obj = __createJavaObj(handle);
    obj.__h = handle;
    // getKey/getTag/getSourceType/getLoginJs 已由 KSP 生成表接管 (注入下方标记处);
    // getHeaderMap 特例保持手写 (推断返回 HashMap 无法归类生成);
    // getLoginUrl/getHeader/getConcurrentRate/getJsLib 为属性 getter 方法化 (阶段 3 E5 接管)
    obj.getHeaderMap = function() {
        var s = __nativeDispatch(handle, 1604, []);
        return (s === null || s === undefined) ? null : JSON.parse(s);
    };
    obj.getLoginUrl = function() { return __nativeDispatch(handle, 1605, []); };
    obj.getHeader = function() { return __nativeDispatch(handle, 1606, []); };
    obj.getConcurrentRate = function() { return __nativeDispatch(handle, 1608, []); };
    obj.getJsLib = function() { return __nativeDispatch(handle, 1609, []); };
    // @@methods:__createBaseSourceObj@@
    // BaseSource 属性 (2000-2099, setter = getter + 1000)
    Object.defineProperty(obj, "concurrentRate", { get: function() { return __nativeDispatch(handle, 2001, []); }, set: function(v) { __nativeDispatch(handle, 3001, [v]); } });
    Object.defineProperty(obj, "loginUrl", { get: function() { return __nativeDispatch(handle, 2002, []); }, set: function(v) { __nativeDispatch(handle, 3002, [v]); } });
    Object.defineProperty(obj, "loginUi", { get: function() { return __nativeDispatch(handle, 2003, []); }, set: function(v) { __nativeDispatch(handle, 3003, [v]); } });
    Object.defineProperty(obj, "header", { get: function() { return __nativeDispatch(handle, 2004, []); }, set: function(v) { __nativeDispatch(handle, 3004, [v]); } });
    Object.defineProperty(obj, "enabledCookieJar", { get: function() { return __nativeDispatch(handle, 2005, []); }, set: function(v) { __nativeDispatch(handle, 3005, [v]); } });
    Object.defineProperty(obj, "enableDangerousApi", { get: function() { return __nativeDispatch(handle, 2006, []); }, set: function(v) { __nativeDispatch(handle, 3006, [v]); } });
    Object.defineProperty(obj, "jsLib", { get: function() { return __nativeDispatch(handle, 2007, []); }, set: function(v) { __nativeDispatch(handle, 3007, [v]); } });
    // BookSource 属性 (1900-1999; 非 BookSource 的 BaseSource 取到 undefined)
    Object.defineProperty(obj, "bookSourceUrl", { get: function() { return __nativeDispatch(handle, 1901, []); }, set: function(v) { __nativeDispatch(handle, 2901, [v]); } });
    Object.defineProperty(obj, "bookSourceName", { get: function() { return __nativeDispatch(handle, 1902, []); }, set: function(v) { __nativeDispatch(handle, 2902, [v]); } });
    Object.defineProperty(obj, "bookSourceGroup", { get: function() { return __nativeDispatch(handle, 1903, []); }, set: function(v) { __nativeDispatch(handle, 2903, [v]); } });
    Object.defineProperty(obj, "bookSourceType", { get: function() { return __nativeDispatch(handle, 1904, []); }, set: function(v) { __nativeDispatch(handle, 2904, [v]); } });
    Object.defineProperty(obj, "bookUrlPattern", { get: function() { return __nativeDispatch(handle, 1905, []); }, set: function(v) { __nativeDispatch(handle, 2905, [v]); } });
    Object.defineProperty(obj, "customOrder", { get: function() { return __nativeDispatch(handle, 1906, []); }, set: function(v) { __nativeDispatch(handle, 2906, [v]); } });
    Object.defineProperty(obj, "enabled", { get: function() { return __nativeDispatch(handle, 1907, []); }, set: function(v) { __nativeDispatch(handle, 2907, [v]); } });
    Object.defineProperty(obj, "enabledExplore", { get: function() { return __nativeDispatch(handle, 1908, []); }, set: function(v) { __nativeDispatch(handle, 2908, [v]); } });
    Object.defineProperty(obj, "enabledReview", { get: function() { return __nativeDispatch(handle, 1909, []); }, set: function(v) { __nativeDispatch(handle, 2909, [v]); } });
    Object.defineProperty(obj, "loginCheckJs", { get: function() { return __nativeDispatch(handle, 1910, []); }, set: function(v) { __nativeDispatch(handle, 2910, [v]); } });
    Object.defineProperty(obj, "coverDecodeJs", { get: function() { return __nativeDispatch(handle, 1911, []); }, set: function(v) { __nativeDispatch(handle, 2911, [v]); } });
    Object.defineProperty(obj, "bookSourceComment", { get: function() { return __nativeDispatch(handle, 1912, []); }, set: function(v) { __nativeDispatch(handle, 2912, [v]); } });
    Object.defineProperty(obj, "variableComment", { get: function() { return __nativeDispatch(handle, 1913, []); }, set: function(v) { __nativeDispatch(handle, 2913, [v]); } });
    Object.defineProperty(obj, "lastUpdateTime", { get: function() { return __nativeDispatch(handle, 1914, []); }, set: function(v) { __nativeDispatch(handle, 2914, [v]); } });
    Object.defineProperty(obj, "respondTime", { get: function() { return __nativeDispatch(handle, 1915, []); }, set: function(v) { __nativeDispatch(handle, 2915, [v]); } });
    Object.defineProperty(obj, "weight", { get: function() { return __nativeDispatch(handle, 1916, []); }, set: function(v) { __nativeDispatch(handle, 2916, [v]); } });
    Object.defineProperty(obj, "exploreUrl", { get: function() { return __nativeDispatch(handle, 1917, []); }, set: function(v) { __nativeDispatch(handle, 2917, [v]); } });
    Object.defineProperty(obj, "exploreScreen", { get: function() { return __nativeDispatch(handle, 1918, []); }, set: function(v) { __nativeDispatch(handle, 2918, [v]); } });
    Object.defineProperty(obj, "exploreStyle", { get: function() { return __nativeDispatch(handle, 1919, []); }, set: function(v) { __nativeDispatch(handle, 2919, [v]); } });
    Object.defineProperty(obj, "ruleExplore", { get: function() { return __nativeDispatch(handle, 1920, []); }, set: function(v) { __nativeDispatch(handle, 2920, [v]); } });
    Object.defineProperty(obj, "searchUrl", { get: function() { return __nativeDispatch(handle, 1921, []); }, set: function(v) { __nativeDispatch(handle, 2921, [v]); } });
    Object.defineProperty(obj, "ruleSearch", { get: function() { return __nativeDispatch(handle, 1922, []); }, set: function(v) { __nativeDispatch(handle, 2922, [v]); } });
    Object.defineProperty(obj, "ruleBookInfo", { get: function() { return __nativeDispatch(handle, 1923, []); }, set: function(v) { __nativeDispatch(handle, 2923, [v]); } });
    Object.defineProperty(obj, "ruleToc", { get: function() { return __nativeDispatch(handle, 1924, []); }, set: function(v) { __nativeDispatch(handle, 2924, [v]); } });
    Object.defineProperty(obj, "ruleContent", { get: function() { return __nativeDispatch(handle, 1925, []); }, set: function(v) { __nativeDispatch(handle, 2925, [v]); } });
    Object.defineProperty(obj, "ruleReview", { get: function() { return __nativeDispatch(handle, 1926, []); }, set: function(v) { __nativeDispatch(handle, 2926, [v]); } });
    return obj;
}

function __createRssJsObj(handle) {
    if (!handle || handle <= 0) return null;
    // RSS 拦截 JS 专属 (contentRule.shouldOverrideUrlLoading): BaseSource 全方法面 +
    // searchBook/addBook (1610/1611); 非 RSS 来源不会注册本工厂, 不影响普通来源 java 绑定
    var obj = __createBaseSourceObj(handle);
    obj.searchBook = function(key) { return __nativeDispatch(handle, 1610, [key]); };
    obj.addBook = function(bookUrl) { return __nativeDispatch(handle, 1611, [bookUrl]); };
    return obj;
}

function __createBookObj(handle) {
    if (!handle || handle <= 0) return null;
    var obj = {};
    obj.__h = handle;
    // 变量/方法面 (2300-2399, 对齐 JVM 反射可见的 RuleDataInterface 默认方法 + Book 方法)
    obj.getVariable = function(key) { return __nativeDispatch(handle, 2301, [key]); };
    obj.putVariable = function(key, value) { return __nativeDispatch(handle, 2302, [key, value]); };
    obj.putBigVariable = function(key, value) { __nativeDispatch(handle, 2303, [key, value]); };
    obj.getBigVariable = function(key) { return __nativeDispatch(handle, 2304, [key]); };
    obj.putCustomVariable = function(value) { __nativeDispatch(handle, 2305, [value]); };
    obj.getCustomVariable = function() { return __nativeDispatch(handle, 2306, []); };
    obj.getKindList = function() { return __nativeDispatch(handle, 2307, []); };
    obj.getRealAuthor = function() { return __nativeDispatch(handle, 2308, []); };
    obj.getReverseToc = function() { return __nativeDispatch(handle, 2309, []); };
    obj.setReverseToc = function(reverse) { __nativeDispatch(handle, 2310, [!!reverse]); };
    // 属性 getter/setter (getter 1701+, setter = getter + 1000)
    Object.defineProperty(obj, "bookUrl", { get: function() { return __nativeDispatch(handle, 1701, []); }, set: function(v) { __nativeDispatch(handle, 2701, [v]); } });
    Object.defineProperty(obj, "name", { get: function() { return __nativeDispatch(handle, 1702, []); }, set: function(v) { __nativeDispatch(handle, 2702, [v]); } });
    Object.defineProperty(obj, "author", { get: function() { return __nativeDispatch(handle, 1703, []); }, set: function(v) { __nativeDispatch(handle, 2703, [v]); } });
    Object.defineProperty(obj, "origin", { get: function() { return __nativeDispatch(handle, 1704, []); }, set: function(v) { __nativeDispatch(handle, 2704, [v]); } });
    Object.defineProperty(obj, "originName", { get: function() { return __nativeDispatch(handle, 1705, []); }, set: function(v) { __nativeDispatch(handle, 2705, [v]); } });
    Object.defineProperty(obj, "kind", { get: function() { return __nativeDispatch(handle, 1706, []); }, set: function(v) { __nativeDispatch(handle, 2706, [v]); } });
    Object.defineProperty(obj, "wordCount", { get: function() { return __nativeDispatch(handle, 1707, []); }, set: function(v) { __nativeDispatch(handle, 2707, [v]); } });
    Object.defineProperty(obj, "variable", { get: function() { return __nativeDispatch(handle, 1708, []); }, set: function(v) { __nativeDispatch(handle, 2708, [v]); } });
    Object.defineProperty(obj, "infoHtml", { get: function() { return __nativeDispatch(handle, 1709, []); }, set: function(v) { __nativeDispatch(handle, 2709, [v]); } });
    Object.defineProperty(obj, "tocHtml", { get: function() { return __nativeDispatch(handle, 1710, []); }, set: function(v) { __nativeDispatch(handle, 2710, [v]); } });
    Object.defineProperty(obj, "type", { get: function() { return __nativeDispatch(handle, 1711, []); }, set: function(v) { __nativeDispatch(handle, 2711, [v]); } });
    Object.defineProperty(obj, "coverUrl", { get: function() { return __nativeDispatch(handle, 1712, []); }, set: function(v) { __nativeDispatch(handle, 2712, [v]); } });
    Object.defineProperty(obj, "intro", { get: function() { return __nativeDispatch(handle, 1713, []); }, set: function(v) { __nativeDispatch(handle, 2713, [v]); } });
    Object.defineProperty(obj, "latestChapterTitle", { get: function() { return __nativeDispatch(handle, 1714, []); }, set: function(v) { __nativeDispatch(handle, 2714, [v]); } });
    Object.defineProperty(obj, "tocUrl", { get: function() { return __nativeDispatch(handle, 1715, []); }, set: function(v) { __nativeDispatch(handle, 2715, [v]); } });
    Object.defineProperty(obj, "originOrder", { get: function() { return __nativeDispatch(handle, 1716, []); }, set: function(v) { __nativeDispatch(handle, 2716, [v]); } });
    // Book 实体属性 (1717-1734)
    Object.defineProperty(obj, "customTag", { get: function() { return __nativeDispatch(handle, 1717, []); }, set: function(v) { __nativeDispatch(handle, 2717, [v]); } });
    Object.defineProperty(obj, "customCoverUrl", { get: function() { return __nativeDispatch(handle, 1718, []); }, set: function(v) { __nativeDispatch(handle, 2718, [v]); } });
    Object.defineProperty(obj, "customIntro", { get: function() { return __nativeDispatch(handle, 1719, []); }, set: function(v) { __nativeDispatch(handle, 2719, [v]); } });
    Object.defineProperty(obj, "charset", { get: function() { return __nativeDispatch(handle, 1720, []); }, set: function(v) { __nativeDispatch(handle, 2720, [v]); } });
    Object.defineProperty(obj, "group", { get: function() { return __nativeDispatch(handle, 1721, []); }, set: function(v) { __nativeDispatch(handle, 2721, [v]); } });
    Object.defineProperty(obj, "latestChapterTime", { get: function() { return __nativeDispatch(handle, 1722, []); }, set: function(v) { __nativeDispatch(handle, 2722, [v]); } });
    Object.defineProperty(obj, "lastCheckTime", { get: function() { return __nativeDispatch(handle, 1723, []); }, set: function(v) { __nativeDispatch(handle, 2723, [v]); } });
    Object.defineProperty(obj, "lastCheckCount", { get: function() { return __nativeDispatch(handle, 1724, []); }, set: function(v) { __nativeDispatch(handle, 2724, [v]); } });
    Object.defineProperty(obj, "totalChapterNum", { get: function() { return __nativeDispatch(handle, 1725, []); }, set: function(v) { __nativeDispatch(handle, 2725, [v]); } });
    Object.defineProperty(obj, "durChapterTitle", { get: function() { return __nativeDispatch(handle, 1726, []); }, set: function(v) { __nativeDispatch(handle, 2726, [v]); } });
    Object.defineProperty(obj, "durChapterIndex", { get: function() { return __nativeDispatch(handle, 1727, []); }, set: function(v) { __nativeDispatch(handle, 2727, [v]); } });
    Object.defineProperty(obj, "durChapterPos", { get: function() { return __nativeDispatch(handle, 1728, []); }, set: function(v) { __nativeDispatch(handle, 2728, [v]); } });
    Object.defineProperty(obj, "durChapterTime", { get: function() { return __nativeDispatch(handle, 1729, []); }, set: function(v) { __nativeDispatch(handle, 2729, [v]); } });
    Object.defineProperty(obj, "canUpdate", { get: function() { return __nativeDispatch(handle, 1730, []); }, set: function(v) { __nativeDispatch(handle, 2730, [v]); } });
    Object.defineProperty(obj, "order", { get: function() { return __nativeDispatch(handle, 1731, []); }, set: function(v) { __nativeDispatch(handle, 2731, [v]); } });
    Object.defineProperty(obj, "syncTime", { get: function() { return __nativeDispatch(handle, 1732, []); }, set: function(v) { __nativeDispatch(handle, 2732, [v]); } });
    Object.defineProperty(obj, "downloadUrls", { get: function() { return __nativeDispatch(handle, 1733, []); }, set: function(v) { __nativeDispatch(handle, 2733, [v]); } });
    Object.defineProperty(obj, "lastChapterIndex", { get: function() { return __nativeDispatch(handle, 1734, []); } }); // val 只读
    return obj;
}

function __createChapterObj(handle) {
    if (!handle || handle <= 0) return null;
    var obj = {};
    obj.__h = handle;
    // 变量/方法面 (2400-2499, 对齐 JVM 反射可见的 RuleDataInterface 默认方法 + BookChapter 方法)
    obj.getVariable = function(key) { return __nativeDispatch(handle, 2401, [key]); };
    obj.putVariable = function(key, value) { return __nativeDispatch(handle, 2402, [key, value]); };
    obj.putBigVariable = function(key, value) { __nativeDispatch(handle, 2403, [key, value]); };
    obj.getBigVariable = function(key) { return __nativeDispatch(handle, 2404, [key]); };
    obj.primaryStr = function() { return __nativeDispatch(handle, 2405, []); };
    obj.getFileName = function() { return __nativeDispatch(handle, 2406, []); };
    obj.getFontName = function() { return __nativeDispatch(handle, 2407, []); };
    // 属性 getter/setter (getter 1801+, setter = getter + 1000)
    Object.defineProperty(obj, "url", { get: function() { return __nativeDispatch(handle, 1801, []); }, set: function(v) { __nativeDispatch(handle, 2801, [v]); } });
    Object.defineProperty(obj, "title", { get: function() { return __nativeDispatch(handle, 1802, []); }, set: function(v) { __nativeDispatch(handle, 2802, [v]); } });
    Object.defineProperty(obj, "bookUrl", { get: function() { return __nativeDispatch(handle, 1803, []); }, set: function(v) { __nativeDispatch(handle, 2803, [v]); } });
    Object.defineProperty(obj, "index", { get: function() { return __nativeDispatch(handle, 1804, []); }, set: function(v) { __nativeDispatch(handle, 2804, [v]); } });
    Object.defineProperty(obj, "isVolume", { get: function() { return __nativeDispatch(handle, 1805, []); }, set: function(v) { __nativeDispatch(handle, 2805, [v]); } });
    Object.defineProperty(obj, "isVip", { get: function() { return __nativeDispatch(handle, 1806, []); }, set: function(v) { __nativeDispatch(handle, 2806, [v]); } });
    Object.defineProperty(obj, "isPay", { get: function() { return __nativeDispatch(handle, 1807, []); }, set: function(v) { __nativeDispatch(handle, 2807, [v]); } });
    Object.defineProperty(obj, "resourceUrl", { get: function() { return __nativeDispatch(handle, 1808, []); }, set: function(v) { __nativeDispatch(handle, 2808, [v]); } });
    Object.defineProperty(obj, "tag", { get: function() { return __nativeDispatch(handle, 1809, []); }, set: function(v) { __nativeDispatch(handle, 2809, [v]); } });
    Object.defineProperty(obj, "wordCount", { get: function() { return __nativeDispatch(handle, 1810, []); }, set: function(v) { __nativeDispatch(handle, 2810, [v]); } });
    Object.defineProperty(obj, "start", { get: function() { return __nativeDispatch(handle, 1811, []); }, set: function(v) { __nativeDispatch(handle, 2811, [v]); } });
    Object.defineProperty(obj, "end", { get: function() { return __nativeDispatch(handle, 1812, []); }, set: function(v) { __nativeDispatch(handle, 2812, [v]); } });
    Object.defineProperty(obj, "variable", { get: function() { return __nativeDispatch(handle, 1813, []); }, set: function(v) { __nativeDispatch(handle, 2813, [v]); } });
    Object.defineProperty(obj, "startFragmentId", { get: function() { return __nativeDispatch(handle, 1814, []); }, set: function(v) { __nativeDispatch(handle, 2814, [v]); } });
    Object.defineProperty(obj, "endFragmentId", { get: function() { return __nativeDispatch(handle, 1815, []); }, set: function(v) { __nativeDispatch(handle, 2815, [v]); } });
    Object.defineProperty(obj, "titleMD5", { get: function() { return __nativeDispatch(handle, 1816, []); }, set: function(v) { __nativeDispatch(handle, 2816, [v]); } });
    return obj;
}

function __createAnalyzeObj(handle) {
    if (!handle || handle <= 0) return null;
    // AnalyzeUrlCore/AnalyzeRuleCore 实现 JsExtensionsCommon, 继承完整方法面
    var obj = __createJavaObj(handle);
    obj.__h = handle;
    Object.defineProperty(obj, "rawUrl", { get: function() { return __nativeDispatch(handle, 2101, []); } }); // val 只读
    Object.defineProperty(obj, "urlNoQuery", { get: function() { return __nativeDispatch(handle, 2102, []); }, set: function(v) { __nativeDispatch(handle, 3102, [v]); } });
    Object.defineProperty(obj, "type", { get: function() { return __nativeDispatch(handle, 2103, []); } }); // val 只读
    Object.defineProperty(obj, "serverID", { get: function() { return __nativeDispatch(handle, 2104, []); } }); // val 只读
    Object.defineProperty(obj, "headerMap", { get: function() {
        var s = __nativeDispatch(handle, 2105, []);
        return (s === null || s === undefined) ? null : JSON.parse(s);
    } }); // val 只读
    Object.defineProperty(obj, "url", { get: function() { return __nativeDispatch(handle, 2106, []); }, set: function(v) { __nativeDispatch(handle, 3106, [v]); } });
    Object.defineProperty(obj, "ruleUrl", { get: function() { return __nativeDispatch(handle, 2107, []); } }); // protected set 两端不可写
    Object.defineProperty(obj, "encodedParams", { get: function() { return __nativeDispatch(handle, 2108, []); }, set: function(v) { __nativeDispatch(handle, 3108, [v]); } });
    // AnalyzeRuleCore 面 (2109-2112; AnalyzeUrlCore 实例取到 undefined/null)
    Object.defineProperty(obj, "chapter", { get: function() { return __createChapterObj(__nativeDispatch(handle, 2109, [])); } });
    Object.defineProperty(obj, "nextChapterUrl", { get: function() { return __nativeDispatch(handle, 2110, []); }, set: function(v) { __nativeDispatch(handle, 3110, [v]); } });
    Object.defineProperty(obj, "variables", { get: function() { return __nativeDispatch(handle, 2111, []); } });
    Object.defineProperty(obj, "ruleData", { get: function() { return __createBookObj(__nativeDispatch(handle, 2112, [])); } });
    return obj;
}

function __createCookieObj(handle) {
    if (!handle || handle <= 0) return null;
    var obj = {};
    obj.__h = handle;
    // SourceNetworkProvider 方法面 (2500-2599, 对齐 JVM 反射可见接口方法)
    obj.getCookie = function(tag) { return __nativeDispatch(handle, 2501, [tag]); };
    obj.replaceCookie = function(tag, cookie) { __nativeDispatch(handle, 2502, [tag, cookie]); };
    obj.removeCookie = function(tag) { __nativeDispatch(handle, 2503, [tag]); };
    return obj;
}

function __createCacheObj(handle) {
    if (!handle || handle <= 0) return null;
    var obj = {};
    obj.__h = handle;
    // SourceCacheProvider 方法面 (2600-2699, 对齐 JVM 反射可见接口方法)
    obj.get = function(key) { return __nativeDispatch(handle, 2601, [key]); };
    obj.put = function(key, value, saveTime) {
        return arguments.length >= 3 ? __nativeDispatch(handle, 2607, [key, value, saveTime]) : __nativeDispatch(handle, 2602, [key, value]);
    };
    obj.delete = function(key) { __nativeDispatch(handle, 2603, [key]); };
    obj.getFromMemory = function(key) { return __nativeDispatch(handle, 2604, [key]); };
    obj.putMemory = function(key, value) { __nativeDispatch(handle, 2605, [key, value]); };
    obj.deleteMemory = function(key) { __nativeDispatch(handle, 2606, [key]); };
    obj.getInt = function(key) { return __nativeDispatch(handle, 2608, [key]); };
    obj.getLong = function(key) { return __nativeDispatch(handle, 2609, [key]); };
    obj.getDouble = function(key) { return __nativeDispatch(handle, 2610, [key]); };
    obj.getFloat = function(key) { return __nativeDispatch(handle, 2611, [key]); };
    obj.clearMemoryByPrefixes = function(prefixes) { __nativeDispatch(handle, 2612, [prefixes]); };
    return obj;
}

function __createElementObj(handle) { return __createJavaCompatObj(handle, false); }

    """.trimIndent()

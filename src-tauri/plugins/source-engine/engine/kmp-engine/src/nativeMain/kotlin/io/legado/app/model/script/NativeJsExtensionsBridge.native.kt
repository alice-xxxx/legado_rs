// 源平台来源：data/src/nativeMain/kotlin/io/legado/app/model/script/NativeJsExtensionsBridge.native.kt。此 actual 已复制进提取模块，构建不读取源平台目录。
@file:OptIn(kotlinx.cinterop.ExperimentalForeignApi::class)

package io.legado.app.model.script

import com.fleeksoft.ksoup.nodes.Node
import com.fleeksoft.ksoup.select.Elements
import io.legado.app.help.JsCryptoProviders
import io.legado.app.data.entities.BaseBook
import io.legado.app.data.entities.BaseSource
import io.legado.app.data.entities.BookChapterLike
import io.legado.app.help.JsExtensionsCommon
import io.legado.app.help.crypto.AsymmetricCrypto
import io.legado.app.help.crypto.Sign
import io.legado.app.help.crypto.SymmetricCrypto
import io.legado.app.help.http.StrResponse
import io.legado.app.help.image.ImageOps
import io.legado.app.help.source.SourceCacheProvider
import io.legado.app.help.source.SourceNetworkProvider
import io.legado.app.model.analyzeRule.AnalyzeRuleCore
import io.legado.app.model.analyzeRule.AnalyzeUrlCore
import io.legado.app.model.analyzeRule.QueryTTF
import io.legado.app.ui.rss.RssJsApi
import io.legado.app.utils.GSON
import io.legado.app.utils.JsURL
import io.legado.app.utils.fromJsonObject
import io.legado.app.utils.toJson
import org.jsoup.Connection
import com.script.jsdispatch.generated.NativeDispatchResult
import com.script.jsdispatch.generated.NativeGeneratedDispatch
import io.legado.app.napi.quickjs.JSContext
import io.legado.app.napi.quickjs.JSValue
import io.legado.app.napi.quickjs.qjs_EvalTypeGlobal
import io.legado.app.napi.quickjs.JS_TAG_NULL
import io.legado.app.napi.quickjs.JS_TAG_UNDEFINED
import io.legado.app.napi.quickjs.JS_FreeValue
import io.legado.app.napi.quickjs.JS_GetLength
import io.legado.app.napi.quickjs.JS_GetPropertyUint32
import io.legado.app.napi.quickjs.JS_IsArray
import io.legado.app.napi.quickjs.JS_NewArray
import io.legado.app.napi.quickjs.JS_NewObject
import io.legado.app.napi.quickjs.JS_SetPropertyStr
import io.legado.app.napi.quickjs.JS_SetPropertyUint32
import io.legado.app.napi.quickjs.JS_Throw
import io.legado.app.napi.quickjs.qjs_IsBool
import io.legado.app.napi.quickjs.qjs_IsNull
import io.legado.app.napi.quickjs.qjs_IsNumber
import io.legado.app.napi.quickjs.qjs_IsString
import io.legado.app.napi.quickjs.qjs_IsUndefined
import io.legado.app.napi.quickjs.qjs_NewBool
import io.legado.app.napi.quickjs.qjs_NewFloat64
import io.legado.app.napi.quickjs.qjs_NewInt32
import io.legado.app.napi.quickjs.qjs_ValueGetBool
import io.legado.app.napi.quickjs.qjs_ValueGetFloat64
import io.legado.app.napi.quickjs.qjs_ValueGetInt
import kotlinx.cinterop.CPointer
import kotlinx.cinterop.CValue
import kotlinx.cinterop.LongVar
import kotlinx.cinterop.alloc
import kotlinx.cinterop.cValue
import kotlinx.cinterop.cstr
import kotlinx.cinterop.get
import kotlinx.cinterop.memScoped
import kotlinx.cinterop.ptr
import kotlinx.cinterop.readValue
import kotlinx.cinterop.staticCFunction
import kotlinx.cinterop.value
import kotlinx.atomicfu.locks.SynchronizedObject
import kotlinx.atomicfu.locks.synchronized

/**
 * native 端 (iOS/鸿蒙) JsExtensions 桥接器: 把 [JsExtensionsCommon] 等 Kotlin 对象桥接为 JS 对象。
 *
 * nativeMain 中间源集下沉: 原 iosMain [IosJsExtensionsBridge] / ohosMain [OhosJsExtensionsBridge]
 * 逻辑完全一致 (基于 cinterop 共享 quickjs 绑定 + 同一份 krypto), 下沉到 nativeMain 共用,
 * iosMain/ohosMain 用 typealias 别名指向本类。
 *
 * # 背景
 * native 端 [NativeJsEngine] P0 阶段 toJsValue 对复杂 Kotlin 对象返回 null (跳过注入),
 * 导致 `bindings["java"] = analyzeRuleCore` 时 JS 里 `java` 为 undefined,
 * `java.createSymmetricCrypto` / `java.md5Encode` 等调用全部失效。
 *
 * # 方案: 通用分派 C 函数 + JS 层工厂包装
 * 1. bootstrap 注入全局 C 函数 `__nativeDispatch(handle, methodId, argsArray)` 到 globalThis;
 * 2. bootstrap 注入 JS 工厂函数 `__createJavaObj(handle)` / `__createCryptoObj(handle)` 等;
 * 3. [NativeJsEngine.toJsValue] 识别 JsExtensionsCommon 时, 注册 handle, 调 `__createJavaObj(handle)` 得到 JS 对象;
 * 4. JS 调用 `java.md5Encode("str")` → JS 层 `__nativeDispatch(handle, 101, ["str"])` → C 回调 → Kotlin 分派。
 *
 * # 为什么不用 JSClassDef + opaque
 * JSClassDef + opaque + StableRef 是更"正规"的方案, 但实现复杂 (finalizer / StableRef 生命周期管理),
 * 且 cinterop 绑定 JSClassDef 结构体的 finalizer 函数指针类型映射容易出错。
 * 本方案用 handle 表 + JS 层闭包捕获 handle, 避免了 C 层面的上下文传递,
 * 只需 1 个 staticCFunction (C 函数不能捕获上下文), 所有方法分派在 Kotlin 层完成。
 *
 * # handle 生命周期
 * - handleTable 用强引用, 防止 Kotlin 对象在 JS 使用期间被 GC;
 * - [NativeJsScope.handles] 记录 scope 内注册的所有 handle, scope close 时统一清理;
 * - createSymmetricCrypto 返回的 SymmetricCrypto 对象 handle 也记录在 scope.handles 中。
 *
 * # 补齐的方法 (methodId)
 * - 1-99: JsExtensionsCommon 纯函数面 (base64Encode/base64Decode/strToBytes/bytesToStr/hex/encodeURI/htmlFormat/timeFormat/randomUUID/t2s/s2t)
 * - 23-28: 规则引擎核心补齐 (getSource/put/get/evalJS/getString/getStringList, 对齐 JVM @JsApi 分派表)
 * - 100-199: JsCryptoProviders 摘要/HMAC 面 (md5Encode/md5Encode16/digestHex/digestBase64Str/HMacHex/HMacBase64)
 * - 200-299: 工厂方法 (createSymmetricCrypto/createAsymmetricCrypto/createSign) 返回 handle (Float64)
 * - 1000-1099: SymmetricCrypto 对象方法 (encryptBase64/decryptStr/decrypt/setIv/encrypt/encryptHex)
 * - 1100-1199: AsymmetricCrypto 对象方法 (setPrivateKey/setPublicKey/decrypt/encrypt/decryptStr/encryptHex/encryptBase64)
 * - 1200-1299: Sign 对象方法 (setPrivateKey/setPublicKey/sign/signHex/verify)
 * - 13-29: 编解码补齐/重载 (strToBytes/bytesToStr/base64 系/hexDecodeToByteArray/encodeURI/timeFormatUTC/toNumChapter/toURL)
 * - 300-399: 网络族 (ajax/ajaxAll/connect/webView 系/getCookie/getWebViewUA; 309-311 java get/head/post)
 * - 400-499: UI/杂项 (refreshUi/log/logType/toast/longToast/androidId/openUrl/startBrowser 系/getVerificationCode/copy)
 * - 500-599: 字体族 (queryTTF/queryBase64TTF/replaceFont)
 * - 600-699: 文件/压缩族 (getFile/readFile/readTxtFile/deleteFile/解压全族/downloadFile/cacheFile/importScript)
 * - 700-799: Connection.Response 对象方法 (java.get/head/post 返回值: body/statusCode/url/header/cookie 等;
 *   规整方法由 KSP 生成表接管; 手写包含 704 url()/712 method() 与 713-717 链式 setter)
 * - 1300-1399: QueryTTF 对象方法 (规整函数手写闭包与生成表并存: 生成注入点在 JS 工厂
 *   @@methods 标记处、位于手写定义之后, 同名以生成为准; 1306-1308 映射表属性为生成器
 *   不覆盖项, 仅手写); 1400-1499: StrResponse 对象方法 (规整已生成表接管,
 *   手写仅 1406 headers toString 降级); 1500-1599: JsURL 对象属性 (仅属性, 生成器无输出, 全手写);
 *   1600-1699: BaseSource 对象方法 (getKey/getTag/getSourceType/getLoginJs 已生成表接管; 手写仅
 *   1604 getHeaderMap 特例与 1605/1606/1608/1609 属性 getter 方法化 —— 属性不在生成范围, 手写保留);
 *   1610-1611: RssJsApi (RSS 拦截 JS 专属 searchBook/addBook, 见 RssJsExtensions.native)
 * - 1700-2199: 复杂对象属性桥 (NativeJsPropertyBridge, book/source/chapter/java 属性 getter,
 *   分派表与 JS 工厂见 NativeJsPropertyBridge.native.kt; >= 1700 在 dispatch 开头短路转发)
 * - 2200+: 带参分派 (NativeJsPropertyBridge.dispatchWithArgs): 2300-2399 book 变量/方法面 |
 *   2400-2499 chapter 变量/方法面 | 2500-2599 cookie (SourceNetworkProvider) |
 *   2600-2699 cache (SourceCacheProvider) | 2700-3199 属性写 (setterId = getterId + 1000) |
 *   3400-3406 JavaClassCompat (类加载、构造、静态成员、对象方法、接口回调) |
 *   3301-3308 ImageOps stateless PNG byte-array bridge (decode/encode/split/stitch/crop/rotate/flip/size)
 * - 5000+: KSP 生成表 (NativeGeneratedDispatch, JsApiProcessor 按 jsapi.nativeTargets 目标类生成;
 *   dispatch 顶部查表优先, JS 闭包按工厂分区注入 `// @@methods:<factory>@@` 标记处)
 *
 * 100-199 摘要段使用 JsCryptoProvider 的公共文本转换；平台只提供字节算法。
 *
 * 注: cookie/cache binding 注入依赖 [NativeJsEngine.toJsValue] 识别 SourceNetworkProvider /
 * SourceCacheProvider (NativeJsEngine.native.kt, 需并行修改方生效; 工厂与分派段已在本文件就绪)。
 */
object NativeJsExtensionsBridge {

    private const val IMAGE_OPS_METHOD_BASE = 3300
    private const val MAX_IMAGE_BRIDGE_BYTES = 64 * 1024 * 1024
    private const val MAX_IMAGE_BASE64_CHARS = ((MAX_IMAGE_BRIDGE_BYTES + 2) / 3) * 4 + 1024
    private const val MAX_IMAGE_STITCH_ITEMS = 32

    /**
     * handle → Kotlin 对象映射 (强引用, 防止 GC)。
     *
     * 全局共享: iOS/鸿蒙各线程独立 scope 并发 eval, register/unregister/lookup 跨线程
     * 访问同一表; K/N 无 ConcurrentHashMap, 所有操作由 [handleTableLock] 串行化。
     */
    private data class Entry(val scope: NativeJsScope, val value: Any)
    private val handleTable = HashMap<Long, Entry>()

    /** handle 表全局锁 (可重入; 临界区只做 map 操作, 不含 JS 回调/IO, 无死锁风险)。 */
    private val handleTableLock = SynchronizedObject()

    /**
     * handle 自增计数器。
     *
     * 在 [handleTableLock] 内自增并入表 (成对原子), 保证跨线程 handle 唯一。
     */
    private var handleCounter: Long = 0

    /** 注册 Kotlin 对象, 返回 handle */
    fun registerObject(obj: Any): Long = synchronized(handleTableLock) {
        val scope = checkNotNull(NativeJsEngine.currentScope()) { "Java object requires an active JS scope" }
        handleCounter += 1
        val handle = handleCounter
        handleTable[handle] = Entry(scope, obj)
        scope.handles.add(handle)
        handle
    }

    /** 取 Kotlin 对象 */
    fun getObject(handle: Long): Any? = synchronized(handleTableLock) {
        handleTable[handle]?.takeIf { it.scope === NativeJsEngine.currentScope() }?.value
    }

    /** 注销 handle (scope close 时调用) */
    fun unregisterObject(handle: Long) {
        synchronized(handleTableLock) {
            handleTable.remove(handle)
        }
    }

    /**
     * 把 JsExtensionsCommon 对象桥接为 JS 对象。
     *
     * 内部: 注册 handle → 调 JS `__createJavaObj(handle)` → 返回 JS 对象。
     * JS 对象的方法通过闭包捕获 handle, 调用 `__nativeDispatch(handle, methodId, argsArray)` 回 Kotlin。
     *
     * @param ctx quickjs JSContext
     * @param ext Kotlin 对象 (JsExtensionsCommon / SymmetricCrypto 等)
     * @param scope 当前 scope (用于记录 handle, close 时清理)
     * @return JS 对象的 JSValue (已 retain, 调用方负责 free 或转移所有权)
     */
    fun createJsObject(
        ctx: CPointer<JSContext>,
        ext: Any,
        scope: NativeJsScope
    ): CValue<JSValue> {
        check(NativeJsEngine.currentScope() === scope) { "Java object requires its active JS scope" }
        val handle = registerObject(ext)
        // 调用 JS 工厂函数 __createJavaObj(handle) 或 __createCryptoObj(handle)
        val factoryFn = when (ext) {
            is ImageOps -> "__createImageOpsObj"
            is SymmetricCrypto -> "__createCryptoObj"
            is AsymmetricCrypto -> "__createAsymCryptoObj"
            is Sign -> "__createSignObj"
            is QueryTTF -> "__createQueryTTFObj"
            is StrResponse -> "__createStrResponseObj"
            is Connection.Response -> "__createJavaCompatObj"
            is JsURL -> "__createJsUrlObj"
            // 复杂对象属性桥 (NativeJsPropertyBridge): 工厂带属性 getter
            // RSS 拦截 JS 专属: RssJsApi 分支须在 BaseSource 之前 (RssJsExtensionsNative 同时是两者),
            // 工厂在 __createBaseSourceObj 基础上叠 searchBook/addBook (方法面分派表见 1610/1611)
            is RssJsApi -> "__createRssJsObj"
            is BaseSource -> "__createBaseSourceObj"
            is BaseBook -> "__createBookObj"
            is BookChapterLike -> "__createChapterObj"
            is AnalyzeUrlCore, is AnalyzeRuleCore -> "__createAnalyzeObj"
            // cookie/cache binding (SourceNetworkProvider/SourceCacheProvider, 注入侧见 NativeJsEngine)
            is SourceNetworkProvider -> "__createCookieObj"
            is SourceCacheProvider -> "__createCacheObj"
            // ksoup Element/Node (src binding 逐项循环场景, 工厂见 NativeJsPropertyBridge)
            is Node -> "__createJavaCompatObj"
            is Elements, is StringBuilder, is Connection -> "__createJavaCompatObj"
            else -> if (JavaClassCompat.supports(ext)) "__createJavaCompatObj" else "__createJavaObj"
        }
        val js = if (factoryFn == "__createJavaCompatObj") "$factoryFn($handle,${JavaClassCompat.isCollection(ext)})" else "$factoryFn($handle)"
        // JS_Eval 执行工厂函数, 返回 JS 对象 (qjsEvalUtf8: input_len 按 UTF-8 字节数传)
        val result = qjsEvalUtf8(ctx, js, "<bridge>", qjs_EvalTypeGlobal())
        // result 是 JS 对象, 调用方负责 JS_FreeValue
        // 但我们在 toJsValue 中返回它, 由 JS_SetPropertyStr 转移所有权
        // 如果 eval 异常, result 是 exception, 调用方会得到异常 JSValue
        // 此处不检查异常 (toJsValue 不检查), 让上层 eval 时暴露
        return result
    }

    // ============ native dispatch C 函数 (staticCFunction, 不捕获上下文) ============

    /**
     * 全局 native 分派函数 C 指针, 由 bootstrap 注入 globalThis.__nativeDispatch。
     *
     * 签名: __nativeDispatch(handle: number, methodId: number, args: array) → any
     * - argv[0] = handle (Float64)
     * - argv[1] = methodId (Int32)
     * - argv[2] = args (JS Array)
     *
     * staticCFunction 限制: 不能捕获上下文, 只能调用 object 方法。
     * 类型由编译器推断为 CPointer<CFunction<(...) -> CValue<JSValue>>>, 与 qjs_NewCFunction 的
     * JSCFunction* 参数兼容 (cinterop 绑定 JSCFunction 为同名函数类型)。
     */
    internal val nativeDispatchFn =
        staticCFunction { ctx: CPointer<JSContext>?, thisVal: CValue<JSValue>, argc: Int, argv: CPointer<JSValue>? ->
            nativeDispatchImpl(ctx, thisVal, argc, argv)
        }

    /**
     * nativeDispatch 的 Kotlin 实现 (object 方法, 可被 staticCFunction 调用)。
     */
    private fun nativeDispatchImpl(
        ctx: CPointer<JSContext>?,
        @Suppress("UNUSED_PARAMETER") thisVal: CValue<JSValue>,
        argc: Int,
        argv: CPointer<JSValue>?
    ): CValue<JSValue> {
        val ctxNotNull = ctx ?: return jsUndefined()
        if (argc < 3 || argv == null) return jsUndefined()
        var imageOpsDispatch = false
        var javaCompatDispatch = false
        try {
            val handle = qjs_ValueGetFloat64(argv[0L].readValue()).toLong()
            val methodId = qjs_ValueGetInt(argv[1L].readValue())
            val argsArray = argv[2L].readValue()
            javaCompatDispatch = methodId in 3400..3406
            if (javaCompatDispatch) {
                val args = jsArrayToList(ctxNotNull, argsArray)
                val result = when (methodId) {
                    3400 -> JavaClassCompat.loadClass(args.getString(0))
                    3401 -> JavaClassCompat.construct((args[0] as Number).toInt(), args[1] as List<Any?>)
                    3402 -> JavaClassCompat.callStatic((args[0] as Number).toInt(), args.getString(1), args[2] as List<Any?>)
                    3404 -> JavaClassCompat.field((args[0] as Number).toInt(), args.getString(1))
                    3405 -> JavaClassCompat.newAdapter((args[0] as Number).toInt(), args[1] as JavaClassCompat.Callback)
                    3406 -> JavaClassCompat.isInterface((args[0] as Number).toInt())
                    else -> {
                        JavaClassCompat.call(getObject(handle) ?: error("Java object was released"), args.getString(0), args[1] as List<Any?>)
                    }
                }
                if (methodId == 3401 && result is String) {
                    val reference = registerObject(result)
                    return qjsEvalUtf8(ctxNotNull, "Object.defineProperty(Object(${GSON.toJson(result)}),'__javaCompatHandle__',{value:$reference})", "<java-string>", qjs_EvalTypeGlobal())
                }
                return NativeJsEngine.toJsValue(ctxNotNull, result) ?: error("Unsupported Java return value")
            }
            val obj = getObject(handle) ?: return jsUndefined()
            imageOpsDispatch = obj is ImageOps &&
                methodId in (IMAGE_OPS_METHOD_BASE + 1)..(IMAGE_OPS_METHOD_BASE + 8)
            return dispatch(ctxNotNull, obj, methodId, argsArray)
        } catch (t: Throwable) {
            if (javaCompatDispatch) {
                val message = t.message?.take(512) ?: "Java compatibility call failed"
                return JS_Throw(ctxNotNull, qjsEvalUtf8(ctxNotNull, "new Error(${GSON.toJson(message)})", "<java-compat>", qjs_EvalTypeGlobal()))
            }
            if (imageOpsDispatch) {
                // Image decode/transform failures must reach the rule's try/catch instead of
                // becoming `undefined` and failing later at an unrelated operation.
                val message = t.message?.take(512) ?: "Image operation failed"
                return JS_Throw(ctxNotNull, nativeJsString(ctxNotNull, message))
            }
            // 桥接异常返回 undefined, 避免 JS 引擎崩溃 (与 Android 端 exotic trap 行为一致)
            return jsUndefined()
        }
    }

    // ============ 方法分派 (Kotlin 层) ============

    /**
     * 根据 methodId 分派到对应 Kotlin 方法。
     *
     * @param ctx quickjs JSContext (用于 JSValue 转换)
     * @param obj Kotlin 对象 (JsExtensionsCommon / SymmetricCrypto 等)
     * @param methodId 方法 ID (见文件头注释)
     * @param argsArray JS Array 参数
     * @return 返回值 JSValue
     */
    fun dispatch(
        ctx: CPointer<JSContext>,
        obj: Any,
        methodId: Int,
        argsArray: CValue<JSValue>
    ): CValue<JSValue> {
        // Handle image calls before the generic argument bridge: image data arrives as a
        // Uint8Array and must not be expanded into a Kotlin List of boxed bytes.
        if (obj is ImageOps && methodId in (IMAGE_OPS_METHOD_BASE + 1)..(IMAGE_OPS_METHOD_BASE + 8)) {
            return dispatchImageOps(ctx, obj, methodId, argsArray)
        }
        // KSP 生成表优先: 未命中 (NONE) 落下方手写分支。生成物为纯 Kotlin (无 cinterop 依赖),
        // 返回值经 [nativeResultToJs] 转 JSValue (与手写分支同层转换语义)。
        // registerHandle: REF 返回白名单方法 (如 Response.parse) 注册对象 handle,
        // JS 层由生成闭包里的工厂函数 (__createElementObj 等) 包装。
        val generated =
            NativeGeneratedDispatch.dispatch(
                obj,
                methodId,
                jsArrayToList(ctx, argsArray),
                ::registerObject
            )
        if (generated !== NativeDispatchResult.NONE) return nativeResultToJs(ctx, generated)
        // 复杂对象属性/方法段 (>= 1700, NativeJsPropertyBridge):
        // 1700-2199 属性 getter 无参数, 直接转发跳过 JS Array 解析 (hot path: 一次属性读取零堆分配);
        // 2200+ 属性写/变量方法/cookie/cache 需解析参数后转发 (setter 与调用路径, 非 hot path)
        if (methodId >= PROPERTY_ID_BASE) {
            return if (methodId < PROPERTY_WRITE_BASE) {
                NativeJsPropertyBridge.dispatch(ctx, obj, methodId) ?: jsUndefined()
            } else {
                val args = jsArrayToList(ctx, argsArray)
                NativeJsPropertyBridge.dispatchWithArgs(ctx, obj, methodId, args) ?: jsUndefined()
            }
        }
        // 把 JS Array 转为 Kotlin List<Any?>
        val args = jsArrayToList(ctx, argsArray)

        return when {
            // ============ JsExtensionsCommon 纯函数面 (1-99) ============
            obj is JsExtensionsCommon && methodId == 1 -> {
                // base64Encode(str)
                stringToJsValue(ctx, obj.base64Encode(args.getString(0)))
            }
            obj is JsExtensionsCommon && methodId == 2 -> {
                // base64Decode(str)
                stringToJsValue(ctx, obj.base64Decode(args.getOrNull(0) as? String))
            }
            obj is JsExtensionsCommon && methodId == 3 -> {
                // strToBytes(str) → ByteArray → JS Array
                byteArrayToJsArray(ctx, obj.strToBytes(args.getString(0)))
            }
            obj is JsExtensionsCommon && methodId == 4 -> {
                // bytesToStr(bytes) → String (bytes 从 JS Array 取)
                val bytes = jsArrayToByteArray(ctx, argsArray)
                stringToJsValue(ctx, obj.bytesToStr(bytes))
            }
            obj is JsExtensionsCommon && methodId == 14 -> {
                // bytesToStr(bytes, charset) — bytes 为 args[0] (JS Array)
                val bytes = args.getOrNull(0).toBytesOrNull() ?: ByteArray(0)
                stringToJsValue(ctx, obj.bytesToStr(bytes, args.getString(1)))
            }
            obj is JsExtensionsCommon && methodId == 15 -> {
                // base64Decode(str, charset|flags) — 第二参类型分派重载
                when (val second = args.getOrNull(1)) {
                    is Number -> stringToJsValue(ctx, obj.base64Decode(args.getString(0), second.toInt()))
                    else -> stringToJsValue(
                        ctx, obj.base64Decode(args.getOrNull(0) as? String, (second as? String) ?: "UTF-8")
                    )
                }
            }
            obj is JsExtensionsCommon && methodId == 16 -> {
                // base64DecodeToByteArray(str[, flags]) → ByteArray?
                val flags = (args.getOrNull(1) as? Number)?.toInt()
                val bytes = if (flags != null) obj.base64DecodeToByteArray(args.getOrNull(0) as? String, flags)
                else obj.base64DecodeToByteArray(args.getOrNull(0) as? String)
                bytes?.let { byteArrayToJsArray(ctx, it) } ?: jsNull()
            }
            obj is JsExtensionsCommon && methodId == 17 -> {
                // base64Encode(str, flags)
                stringToJsValue(ctx, obj.base64Encode(args.getString(0), (args.getOrNull(1) as? Number)?.toInt() ?: 2))
            }
            obj is JsExtensionsCommon && methodId == 23 -> {
                // getSource() → BaseSource handle (0 = null, JS 层 __createBaseSourceObj 包装)
                val source = when (obj) {
                    is AnalyzeRuleCore -> obj.getSource()
                    is AnalyzeUrlCore -> obj.getSource()
                    is BaseSource -> obj // BaseSource.getSource() 返回自身 (与 JVM 行为一致)
                    else -> null
                }
                qjs_NewFloat64(ctx, (source?.let { registerObject(it) } ?: 0L).toDouble())
            }

            obj is JsExtensionsCommon && methodId == 24 -> {
                // put(key, value) → value (AnalyzeRuleCore/AnalyzeUrlCore 共用, 保存变量)
                stringToJsValue(ctx, obj.corePut(args.getString(0), args.getString(1)))
            }

            obj is JsExtensionsCommon && methodId == 25 -> {
                // get(key) → String (读取变量)
                stringToJsValue(ctx, obj.coreGet(args.getString(0)))
            }

            obj is JsExtensionsCommon && methodId == 26 -> {
                // evalJS(jsStr) → 任意值 (String/Number/Boolean/Map/List → JS 值)
                val result = obj.coreEvalJS(args.getString(0))
                anyToJs(ctx, result)
            }

            obj is AnalyzeRuleCore && methodId == 27 -> {
                // getString(rule, content[, isUrl]) → String (规则解析, 与 JVM AnalyzeRule.getString 对齐)
                val isUrl = (args.getOrNull(2) as? Boolean) ?: false
                stringToJsValue(
                    ctx,
                    obj.getString(args.getOrNull(0) as? String, args.getOrNull(1), isUrl)
                )
            }

            obj is AnalyzeRuleCore && methodId == 28 -> {
                // getStringList(rule, content[, isUrl]) → String[] (null → JS null)
                val isUrl = (args.getOrNull(2) as? Boolean) ?: false
                val list = obj.getStringList(args.getOrNull(0) as? String, args.getOrNull(1), isUrl)
                if (list == null) {
                    jsNull()
                } else {
                    val arr = JS_NewArray(ctx)
                    list.forEachIndexed { i, s ->
                        JS_SetPropertyUint32(ctx, arr, i.toUInt(), stringToJsValue(ctx, s))
                    }
                    arr
                }
            }

            // ============ JsCryptoProviders 摘要/HMAC 面 (100-199) ============
            // 平台实现在 HostRuntime 注册 (JsCryptoProviderNative, 复用 MD5Utils /
            // NativeDigestOps/NativeHmacOps), 与 JsExtensionsCommon 默认方法同源, 保证 java.md5Encode 等与 Android 端同结果
            methodId in 101..106 -> {
                val enc = JsCryptoProviders.get()
                when (methodId) {
                    101 -> stringToJsValue(ctx, enc.md5Encode(args.getString(0)))
                    102 -> stringToJsValue(ctx, enc.md5Encode16(args.getString(0)))
                    103 -> stringToJsValue(ctx, enc.digestHex(args.getString(0), args.getString(1)))
                    104 -> stringToJsValue(ctx, enc.digestBase64Str(args.getString(0), args.getString(1)))
                    105 -> stringToJsValue(ctx, enc.HMacHex(args.getString(0), args.getString(1), args.getString(2)))
                    106 -> stringToJsValue(ctx, enc.HMacBase64(args.getString(0), args.getString(1), args.getString(2)))
                    else -> jsUndefined()
                }
            }

            // ============ 工厂方法 (200-299) → 返回 handle (Float64) ============
            obj is JsExtensionsCommon && methodId == 201 -> {
                val crypto = obj.createSymmetricCrypto(
                    args.getString(0), args.getOrNull(1).toBytesOrNull(), args.getOrNull(2).toBytesOrNull()
                )
                qjs_NewFloat64(ctx, registerObject(crypto).toDouble())
            }
            obj is JsExtensionsCommon && methodId == 202 ->
                qjs_NewFloat64(ctx, registerObject(obj.createAsymmetricCrypto(args.getString(0))).toDouble())
            obj is JsExtensionsCommon && methodId == 203 ->
                qjs_NewFloat64(ctx, registerObject(obj.createSign(args.getString(0))).toDouble())

            // ============ SymmetricCrypto 对象方法 (1000-1099) ============
            obj is SymmetricCrypto && methodId == 1001 -> {
                // encryptBase64(data) — data 支持 Utf8String|ByteArray
                when (val data = args.getOrNull(0)) {
                    is String -> stringToJsValue(ctx, if (args.size > 1) obj.encryptBase64(data, args.getOrNull(1) as? String) else obj.encryptBase64(data))
                    else -> stringToJsValue(ctx, obj.encryptBase64(data.toBytesOrNull() ?: ByteArray(0)))
                }
            }
            obj is SymmetricCrypto && methodId == 1002 -> {
                // decryptStr(data) → String
                val data = args.getString(0)
                stringToJsValue(ctx, obj.decrypt(data).decodeToString())
            }
            obj is SymmetricCrypto && methodId == 1003 -> {
                // decrypt(data) → ByteArray → JS Array
                val data = args.getString(0)
                byteArrayToJsArray(ctx, obj.decrypt(data))
            }
            obj is SymmetricCrypto && methodId == 1004 -> {
                // setIv(iv) → 真实存 IV (iv 支持 Utf8String|ByteArray); JS 层返回 this
                args.getOrNull(0).toBytesOrNull()?.let { obj.setIv(it) }
                jsUndefined()
            }
            obj is SymmetricCrypto && methodId == 1005 -> {
                // encrypt(data) → ByteArray → JS Array — data 支持 Utf8String|ByteArray
                when (val data = args.getOrNull(0)) {
                    is String -> byteArrayToJsArray(ctx, obj.encrypt(data))
                    else -> byteArrayToJsArray(ctx, obj.encrypt(data.toBytesOrNull() ?: ByteArray(0)))
                }
            }
            obj is SymmetricCrypto && methodId == 1006 -> {
                // encryptHex(data) — data 支持 Utf8String|ByteArray
                when (val data = args.getOrNull(0)) {
                    is String -> stringToJsValue(ctx, obj.encryptHex(data))
                    else -> stringToJsValue(ctx, obj.encryptHex(data.toBytesOrNull() ?: ByteArray(0)))
                }
            }

            // ============ AsymmetricCrypto 对象方法 (1100-1199, iOS Security.framework / 鸿蒙 napi 桥接) ============
            obj is AsymmetricCrypto && methodId == 1101 -> {
                // setPrivateKey(key) → this
                obj.setPrivateKey(requireNotNull(args.getOrNull(0).toBytesOrNull()))
                qjs_NewFloat64(ctx, 1.0) // JS 层返回 this, 此处不实际使用
            }
            obj is AsymmetricCrypto && methodId == 1102 -> {
                // setPublicKey(key) → this
                obj.setPublicKey(requireNotNull(args.getOrNull(0).toBytesOrNull()))
                qjs_NewFloat64(ctx, 1.0)
            }
            obj is AsymmetricCrypto && methodId == 1103 -> {
                // decrypt(data, usePublicKey) → ByteArray (iOS 真实 / 鸿蒙抛异常)
                val usePublicKey = args.getOrNull(1) as? Boolean
                byteArrayToJsArray(ctx, obj.decrypt(args.getDataArg(0), usePublicKey))
            }
            obj is AsymmetricCrypto && methodId == 1104 -> {
                // encrypt(data, usePublicKey) → ByteArray (iOS 真实 / 鸿蒙抛异常)
                val usePublicKey = args.getOrNull(1) as? Boolean
                byteArrayToJsArray(ctx, obj.encrypt(args.getDataArg(0), usePublicKey))
            }
            obj is AsymmetricCrypto && methodId == 1105 -> {
                // decryptStr(data, usePublicKey) — data String 走 hex 优先否则 base64 (hutool SecureUtil.decode)
                val usePublicKey = args.getOrNull(1) as? Boolean
                stringToJsValue(ctx, obj.decryptStr(args.getDataArg(0), usePublicKey))
            }
            obj is AsymmetricCrypto && methodId == 1106 -> {
                // encryptHex(data, usePublicKey)
                val usePublicKey = args.getOrNull(1) as? Boolean
                stringToJsValue(ctx, obj.encryptHex(args.getDataArg(0), usePublicKey))
            }
            obj is AsymmetricCrypto && methodId == 1107 -> {
                // encryptBase64(data, usePublicKey)
                val usePublicKey = args.getOrNull(1) as? Boolean
                stringToJsValue(ctx, obj.encryptBase64(args.getDataArg(0), usePublicKey))
            }

            // ============ Sign 对象方法 (1200-1299, iOS Security.framework 真实 / 鸿蒙 napi) ============
            obj is Sign && methodId == 1201 -> {
                // setPrivateKey(key) → this
                obj.setPrivateKey(requireNotNull(args.getOrNull(0).toBytesOrNull()))
                qjs_NewFloat64(ctx, 1.0)
            }
            obj is Sign && methodId == 1202 -> {
                // setPublicKey(key) → this
                obj.setPublicKey(requireNotNull(args.getOrNull(0).toBytesOrNull()))
                qjs_NewFloat64(ctx, 1.0)
            }
            obj is Sign && methodId == 1203 -> {
                // sign(data) → ByteArray → JS Array — data 支持 Utf8String|ByteArray
                when (val data = args.getDataArg(0)) {
                    is String -> byteArrayToJsArray(ctx, obj.sign(data))
                    is ByteArray -> byteArrayToJsArray(ctx, obj.sign(data))
                    else -> jsUndefined()
                }
            }
            obj is Sign && methodId == 1204 -> {
                // signHex(data) — 小写 hex (对齐 hutool HexUtil.encodeHexStr)
                when (val data = args.getDataArg(0)) {
                    is String -> stringToJsValue(ctx, obj.signHex(data))
                    is ByteArray -> stringToJsValue(ctx, obj.signHex(data))
                    else -> jsUndefined()
                }
            }
            obj is Sign && methodId == 1205 -> {
                // verify(data, sign) → Boolean
                val sig = args.getOrNull(1).toBytesOrNull() ?: ByteArray(0)
                val result = when (val data = args.getDataArg(0)) {
                    is String -> obj.verify(data, sig)
                    is ByteArray -> obj.verify(data, sig)
                    else -> false
                }
                qjs_NewBool(ctx, if (result) 1 else 0)
            }

            // ============ 网络族 (300-399) ============
            obj is JsExtensionsCommon && methodId == 301 -> {
                // ajax(url[, timeoutMs]) — url 支持 String | Array，超时单位为毫秒。
                val url = args.getOrNull(0) ?: ""
                val response = if (args.size < 2) {
                    obj.ajax(url)
                } else {
                    val timeoutMs = (args[1] as? Number)?.toLong()
                        ?: error("ajax timeoutMs must be a number")
                    obj.ajax(url, timeoutMs)
                }
                stringToJsValue(ctx, response)
            }
            obj is JsExtensionsCommon && methodId == 302 -> {
                // ajaxAll(urlList) → StrResponse handle 数组 (JS 层逐个包装)
                val urls = (args.getOrNull(0) as? List<*>)?.map { it.toString() }?.toTypedArray()
                    ?: emptyArray()
                val responses = obj.ajaxAll(urls)
                val arr = JS_NewArray(ctx)
                responses.forEachIndexed { i, resp ->
                    JS_SetPropertyUint32(
                        ctx, arr, i.toUInt(),
                        qjs_NewFloat64(ctx, registerObject(resp).toDouble())
                    )
                }
                arr
            }
            obj is JsExtensionsCommon && methodId == 303 -> {
                // connect(urlStr[, header]) → StrResponse handle (header null 时与单参重载等价)
                val resp = obj.connect(args.getString(0), args.getOrNull(1) as? String)
                qjs_NewFloat64(ctx, registerObject(resp).toDouble())
            }
            obj is JsExtensionsCommon && methodId == 304 -> {
                stringToJsValue(ctx, obj.webView(args.getStringOrNull(0), args.getStringOrNull(1), args.getStringOrNull(2), args.webDelay(3)))
            }
            obj is JsExtensionsCommon && methodId == 305 -> {
                stringToJsValue(ctx, obj.webViewGetSource(args.getStringOrNull(0), args.getStringOrNull(1), args.getStringOrNull(2), args.getStringOrNull(3), args.webDelay(4)))
            }
            obj is JsExtensionsCommon && methodId == 306 -> {
                stringToJsValue(ctx, obj.webViewGetOverrideUrl(args.getStringOrNull(0), args.getStringOrNull(1), args.getStringOrNull(2), args.getStringOrNull(3), args.webDelay(4)))
            }
            obj is JsExtensionsCommon && methodId == 307 -> {
                // getCookie(tag[, key])
                stringToJsValue(ctx, obj.getCookie(args.getString(0), args.getOrNull(1) as? String))
            }
            obj is JsExtensionsCommon && methodId == 309 -> {
                // java.get(url, headers[, timeoutMs]) — 单参 java.get(key) 走 25 变量读取。
                // headers 由 JS 层 JSON.stringify (对象/JSON 字符串都转成 JSON 文本), 此处解析回 Map
                val headerMap = parseHeaderArg(args.getOrNull(1))
                val response = if (args.size < 3) {
                    obj.get(args.getString(0), headerMap)
                } else {
                    val timeoutMs = (args[2] as? Number)?.toInt()
                        ?: error("get timeoutMs must be a number")
                    obj.get(args.getString(0), headerMap, timeoutMs)
                }
                qjs_NewFloat64(ctx, registerObject(response).toDouble())
            }
            obj is JsExtensionsCommon && methodId == 310 -> {
                // java.head(url, headers)
                val headerMap = parseHeaderArg(args.getOrNull(1))
                qjs_NewFloat64(ctx, registerObject(obj.head(args.getString(0), headerMap)).toDouble())
            }
            obj is JsExtensionsCommon && methodId == 311 -> {
                // java.post(url, body, headers[, timeoutMs])
                val headerMap = parseHeaderArg(args.getOrNull(2))
                val response = if (args.size < 4) {
                    obj.post(args.getString(0), args.getString(1), headerMap)
                } else {
                    val timeoutMs = (args[3] as? Number)?.toInt()
                        ?: error("post timeoutMs must be a number")
                    obj.post(args.getString(0), args.getString(1), headerMap, timeoutMs)
                }
                qjs_NewFloat64(ctx, registerObject(response).toDouble())
            }

            // ============ Connection.Response 对象方法 (700-799, java.get/head/post 返回值) ============
            // 规整方法已由 KSP 生成表接管 (5000+ 段); 以下为无法模板化的特例:
            // 704 url() URL→String; 712 method() 枚举 .name; 713-719 Base 请求头/cookie 方法
            obj is Connection.Response && methodId == 704 -> stringToJsValue(ctx, obj.url()?.toString())
            obj is Connection.Response && methodId == 712 -> stringToJsValue(ctx, obj.method().name)
            obj is Connection.Base<*> && methodId == 718 -> {
                // header(name) 读取请求头
                stringToJsValue(ctx, obj.header(args.getString(0)))
            }
            obj is Connection.Base<*> && methodId == 713 -> {
                // header(name, value) 设置请求头
                obj.header(args.getString(0), args.getOrNull(1) as? String)
                jsUndefined()
            }
            obj is Connection.Base<*> && methodId == 714 -> {
                // addHeader(name, value) 追加请求头
                obj.addHeader(args.getString(0), args.getOrNull(1) as? String)
                jsUndefined()
            }
            obj is Connection.Base<*> && methodId == 715 -> {
                // removeHeader(name) 删除请求头
                obj.removeHeader(args.getString(0))
                jsUndefined()
            }
            obj is Connection.Base<*> && methodId == 719 -> {
                // cookie(name) 读取 cookie
                stringToJsValue(ctx, obj.cookie(args.getString(0)))
            }
            obj is Connection.Base<*> && methodId == 716 -> {
                // cookie(name, value) 设置 cookie
                obj.cookie(args.getString(0), args.getString(1))
                jsUndefined()
            }
            obj is Connection.Base<*> && methodId == 717 -> {
                // removeCookie(name) 删除 cookie
                obj.removeCookie(args.getString(0))
                jsUndefined()
            }

            obj is JsExtensionsCommon && methodId == 403 -> {
                // logType(any)
                obj.logType(args.getOrNull(0))
                jsUndefined()
            }
            obj is JsExtensionsCommon && methodId == 408 -> {
                // startBrowser(url, title)
                obj.startBrowser(args.getString(0), args.getString(1))
                jsUndefined()
            }
            obj is JsExtensionsCommon && methodId == 409 -> {
                // startBrowserAwait(url, title[, refetchAfterSuccess]) → StrResponse handle
                val refetch = (args.getOrNull(2) as? Boolean) ?: false
                val resp = obj.startBrowserAwait(args.getString(0), args.getString(1), refetch)
                qjs_NewFloat64(ctx, registerObject(resp).toDouble())
            }
            obj is JsExtensionsCommon && methodId == 414 -> stringToJsValue(ctx, obj.deviceID())
            obj is JsExtensionsCommon && methodId == 415 -> { obj.refreshExplore(); jsUndefined() }
            obj is JsExtensionsCommon && methodId == 416 -> {
                val data = io.legado.app.utils.KS_JSON.parseToJsonElement(args.getString(0)) as kotlinx.serialization.json.JsonObject
                obj.upLoginData(data.mapValues { (_, value) -> (value as? kotlinx.serialization.json.JsonPrimitive)?.content })
                jsUndefined()
            }
            obj is JsExtensionsCommon && methodId == 417 -> {
                obj.reLoginView((args.getOrNull(0) as? Boolean) ?: false)
                jsUndefined()
            }
            // ============ 字体族 (500-599) ============
            obj is JsExtensionsCommon && methodId == 501 -> {
                // queryTTF(data[, useCache]) → QueryTTF handle (null → 0, JS 层转 null)
                val data = args.getDataArgOrNull(0)
                val useCache = args.getOrNull(1) as? Boolean
                val ttf = obj.queryTTF(data, useCache)
                qjs_NewFloat64(ctx, (ttf?.let { registerObject(it) } ?: 0L).toDouble())
            }
            obj is JsExtensionsCommon && methodId == 502 -> {
                // queryBase64TTF(data) — 已过时, 转发保持书源兼容
                @Suppress("DEPRECATION")
                val ttf = obj.queryBase64TTF(args.getOrNull(0) as? String)
                qjs_NewFloat64(ctx, (ttf?.let { registerObject(it) } ?: 0L).toDouble())
            }
            obj is JsExtensionsCommon && methodId == 503 -> {
                // replaceFont(text, errTTFHandle, corTTFHandle[, filter]) — handle 由 JS 包装层传回
                val err = (args.getOrNull(1) as? Number)?.toLong()?.let { getObject(it) as? QueryTTF }
                val cor = (args.getOrNull(2) as? Number)?.toLong()?.let { getObject(it) as? QueryTTF }
                val filter = (args.getOrNull(3) as? Boolean) ?: false
                stringToJsValue(ctx, obj.replaceFont(args.getString(0), err, cor, filter))
            }

            obj is JsExtensionsCommon && methodId == 611 -> {
                // getRarStringContent(url, path[, charsetName])
                val charset = args.getOrNull(2) as? String
                val text = if (charset != null) {
                    obj.getRarStringContent(args.getString(0), args.getString(1), charset)
                } else obj.getRarStringContent(args.getString(0), args.getString(1))
                stringToJsValue(ctx, text)
            }
            obj is JsExtensionsCommon && methodId == 612 -> {
                // get7zStringContent(url, path[, charsetName])
                val charset = args.getOrNull(2) as? String
                val text = if (charset != null) {
                    obj.get7zStringContent(args.getString(0), args.getString(1), charset)
                } else obj.get7zStringContent(args.getString(0), args.getString(1))
                stringToJsValue(ctx, text)
            }
            obj is JsExtensionsCommon && methodId == 617 -> {
                // cacheFile(urlStr[, saveTime])
                val saveTime = (args.getOrNull(1) as? Number)?.toInt()
                val text = if (saveTime != null) obj.cacheFile(args.getString(0), saveTime)
                else obj.cacheFile(args.getString(0))
                stringToJsValue(ctx, text)
            }
            // ============ QueryTTF 对象方法 (1300-1399) ============
            // getGlyfById 等规整函数已由 KSP 生成表接管 (5000+ 段, JS 闭包经 @@methods 注入点覆盖);
            // 此处仅保留 1306-1308 映射表属性 (生成器只收集函数, 属性不生成)
            obj is QueryTTF && methodId == 1301 -> {
                // getGlyfById(glyfId) → String?
                stringToJsValue(ctx, obj.getGlyfById((args.getOrNull(0) as? Number)?.toInt() ?: 0))
            }
            obj is QueryTTF && methodId == 1302 -> {
                // getGlyfIdByUnicode(unicode) → Int
                qjs_NewInt32(ctx, obj.getGlyfIdByUnicode((args.getOrNull(0) as? Number)?.toInt() ?: 0))
            }
            obj is QueryTTF && methodId == 1303 -> {
                // getGlyfByUnicode(unicode) → String?
                stringToJsValue(ctx, obj.getGlyfByUnicode((args.getOrNull(0) as? Number)?.toInt() ?: 0))
            }
            obj is QueryTTF && methodId == 1304 -> {
                // getUnicodeByGlyf(glyph) → Int
                qjs_NewInt32(ctx, obj.getUnicodeByGlyf(args.getOrNull(0) as? String))
            }
            obj is QueryTTF && methodId == 1305 -> {
                // isBlankUnicode(unicode) → Boolean
                qjs_NewBool(ctx, if (obj.isBlankUnicode((args.getOrNull(0) as? Number)?.toInt() ?: 0)) 1 else 0)
            }
            obj is QueryTTF && methodId == 1306 -> {
                // unicodeToGlyph 映射表 → JSON (JS 层 JSON.parse 后惰性缓存)
                stringToJsValue(ctx, GSON.toJson(obj.unicodeToGlyph))
            }
            obj is QueryTTF && methodId == 1307 -> {
                // glyphToUnicode 映射表 → JSON
                stringToJsValue(ctx, GSON.toJson(obj.glyphToUnicode))
            }
            obj is QueryTTF && methodId == 1308 -> {
                // unicodeToGlyphId 映射表 → JSON
                stringToJsValue(ctx, GSON.toJson(obj.unicodeToGlyphId))
            }

            // ============ StrResponse 对象方法 (1400-1499) ============
            // 规整方法已由 KSP 生成表接管; 1406 特例: headers() 降级为字符串
            // (native KmpHeaders 无完整对象桥, 与 Response.headers() 的 JSON 语义不同)
            obj is StrResponse && methodId == 1406 -> {
                stringToJsValue(ctx, obj.headers().toString())
            }

            // ============ JsURL 对象属性 (1500-1599) ============
            obj is JsURL && methodId == 1501 -> stringToJsValue(ctx, obj.host)
            obj is JsURL && methodId == 1502 -> stringToJsValue(ctx, obj.origin)
            obj is JsURL && methodId == 1503 -> stringToJsValue(ctx, obj.pathname)
            obj is JsURL && methodId == 1504 -> {
                // searchParams → JSON (null → JS null)
                obj.searchParams?.let { stringToJsValue(ctx, GSON.toJson(it)) } ?: jsNull()
            }

            // ============ BaseSource 对象方法 (1600-1699, getSource() 返回对象) ============
            // getKey/getTag/getSourceType/getLoginJs 已由 KSP 生成表接管; 1604 特例: getHeaderMap
            // 推断返回 HashMap 无法归类生成 (名单内, 手写 JSON);
            // 1605/1606/1608/1609 为属性 getter 方法化 (阶段 3 E5 属性遍历接管)
            obj is BaseSource && methodId == 1604 -> {
                // getHeaderMap() → JSON 对象 (返回类型非空, 无 null 分支)
                stringToJsValue(ctx, GSON.toJson(obj.getHeaderMap()))
            }

            obj is BaseSource && methodId == 1605 -> stringToJsValue(ctx, obj.loginUrl)
            obj is BaseSource && methodId == 1606 -> stringToJsValue(ctx, obj.header)
            obj is BaseSource && methodId == 1608 -> stringToJsValue(ctx, obj.concurrentRate)
            obj is BaseSource && methodId == 1609 -> stringToJsValue(ctx, obj.jsLib)

            // ============ RssJsApi 对象方法 (1610-1611, RSS 拦截 JS 专属) ============
            // RssJsExtensionsNative 包装 (BaseSource + RssJsApi) 经 __createRssJsObj 注入;
            // 非 RSS 上下文的 BaseSource 对象不含这两个方法, 不会走到此分支
            obj is RssJsApi && methodId == 1610 -> {
                // searchBook(key)
                obj.searchBook(args.getString(0))
                jsUndefined()
            }
            obj is RssJsApi && methodId == 1611 -> {
                // addBook(bookUrl)
                obj.addBook(args.getString(0))
                jsUndefined()
            }

            else -> jsUndefined()
        }
    }

    private fun dispatchImageOps(
        ctx: CPointer<JSContext>,
        imageOps: ImageOps,
        methodId: Int,
        argsArray: CValue<JSValue>
    ): CValue<JSValue> {
        fun imageInput(index: Int): io.legado.app.help.image.ImageRef =
            withArgument(ctx, argsArray, index) { value ->
                if (qjs_IsString(value) != 0) {
                    val base64 = jsValueToString(ctx, value)
                    require(base64.length <= MAX_IMAGE_BASE64_CHARS) {
                        "Image base64 input exceeds the 64 MiB decoded limit"
                    }
                    imageOps.decode(base64)
                } else {
                    imageOps.decode(imageBytes(ctx, value))
                }
            }

        fun imageArgument(index: Int): io.legado.app.help.image.ImageRef =
            imageOps.decode(withArgument(ctx, argsArray, index) { imageBytes(ctx, it) })

        fun pngBytes(image: io.legado.app.help.image.ImageRef): ByteArray =
            imageOps.encode(image, "png", 100)

        fun dimension(index: Int): Int = withArgument(ctx, argsArray, index) { value ->
            require(qjs_IsNumber(value) != 0) { "Image operation dimension must be a number" }
            qjs_ValueGetFloat64(value).toInt()
        }

        fun text(index: Int): String = withArgument(ctx, argsArray, index) { value ->
            require(qjs_IsString(value) != 0) { "Image operation option must be a string" }
            jsValueToString(ctx, value)
        }

        return when (methodId - IMAGE_OPS_METHOD_BASE) {
            1 -> NativeJsEngine.byteArrayToJsUint8Array(ctx, pngBytes(imageInput(0)))
            2 -> {
                val image = imageArgument(0)
                val format = text(1)
                val quality = dimension(2)
                NativeJsEngine.byteArrayToJsUint8Array(
                    ctx,
                    imageOps.encode(image, format, quality)
                )
            }
            3 -> {
                val image = imageArgument(0)
                val rows = dimension(1)
                val columns = dimension(2)
                val pngImages = imageOps.split(image, rows, columns).map(::pngBytes)
                val result = JS_NewArray(ctx)
                pngImages.forEachIndexed { index, bytes ->
                    JS_SetPropertyUint32(
                        ctx,
                        result,
                        index.toUInt(),
                        NativeJsEngine.byteArrayToJsUint8Array(ctx, bytes)
                    )
                }
                result
            }
            4 -> {
                val inputs = imageList(ctx, imageOps, argsArray, 0)
                val direction = text(1)
                NativeJsEngine.byteArrayToJsUint8Array(
                    ctx,
                    pngBytes(imageOps.stitch(inputs, direction))
                )
            }
            5 -> {
                val image = imageArgument(0)
                val crop = imageOps.crop(image, dimension(1), dimension(2), dimension(3), dimension(4))
                NativeJsEngine.byteArrayToJsUint8Array(ctx, pngBytes(crop))
            }
            6 -> {
                val image = imageArgument(0)
                NativeJsEngine.byteArrayToJsUint8Array(
                    ctx,
                    pngBytes(imageOps.rotate(image, dimension(1)))
                )
            }
            7 -> {
                val image = imageArgument(0)
                NativeJsEngine.byteArrayToJsUint8Array(
                    ctx,
                    pngBytes(imageOps.flip(image, text(1)))
                )
            }
            8 -> anyToJs(ctx, imageOps.size(imageArgument(0)))
            else -> jsUndefined()
        }
    }

    private inline fun <T> withArgument(
        ctx: CPointer<JSContext>,
        args: CValue<JSValue>,
        index: Int,
        block: (CValue<JSValue>) -> T
    ): T {
        val value = JS_GetPropertyUint32(ctx, args, index.toUInt())
        return try {
            block(value)
        } finally {
            JS_FreeValue(ctx, value)
        }
    }

    private fun imageBytes(ctx: CPointer<JSContext>, value: CValue<JSValue>): ByteArray {
        NativeJsEngine.tryGetUint8ArrayBytes(ctx, value, MAX_IMAGE_BRIDGE_BYTES)?.let {
            return it
        }
        require(JS_IsArray(value)) { "Image input must be base64 or a byte array" }
        val length = arrayLength(ctx, value, MAX_IMAGE_BRIDGE_BYTES)
        return ByteArray(length) { index ->
            val item = JS_GetPropertyUint32(ctx, value, index.toUInt())
            try {
                require(qjs_IsNumber(item) != 0) { "Image byte array must contain numbers" }
                qjs_ValueGetFloat64(item).toInt().toByte()
            } finally {
                JS_FreeValue(ctx, item)
            }
        }
    }

    private fun imageList(
        ctx: CPointer<JSContext>,
        imageOps: ImageOps,
        args: CValue<JSValue>,
        index: Int
    ): List<io.legado.app.help.image.ImageRef> = withArgument(ctx, args, index) { value ->
        require(JS_IsArray(value)) { "image.stitch expects an array of images" }
        val count = arrayLength(ctx, value, MAX_IMAGE_STITCH_ITEMS)
        List(count) { itemIndex ->
            val item = JS_GetPropertyUint32(ctx, value, itemIndex.toUInt())
            try {
                imageOps.decode(imageBytes(ctx, item))
            } finally {
                JS_FreeValue(ctx, item)
            }
        }
    }

    private fun arrayLength(
        ctx: CPointer<JSContext>,
        array: CValue<JSValue>,
        maximum: Int
    ): Int = memScoped {
        val length = alloc<LongVar>()
        require(JS_GetLength(ctx, array, length.ptr) == 0) { "Image input has no byte length" }
        require(length.value in 1..maximum.toLong()) { "Image input exceeds its size limit" }
        length.value.toInt()
    }

    // ============ JS 工厂代码 (注入 bootstrap) ============

    val JS_FACTORY_CODE = nativeBindingFactories(NativeGeneratedDispatch.JS_METHOD_TABLES)

    // ============ 工具方法: JSValue ↔ Kotlin 转换 ============

    /** KSP 生成表 (NativeGeneratedDispatch) 的返回值: 纯 Kotlin, 由 [nativeResultToJs] 转 JSValue。 */
    private fun nativeResultToJs(
        ctx: CPointer<JSContext>,
        r: NativeDispatchResult
    ): CValue<JSValue> = when (r) {
        is NativeDispatchResult.Str -> stringToJsValue(ctx, r.v)
        is NativeDispatchResult.Int -> qjs_NewInt32(ctx, r.v)
        is NativeDispatchResult.Long -> qjs_NewFloat64(ctx, r.v.toDouble())
        is NativeDispatchResult.Double -> qjs_NewFloat64(ctx, r.v)
        is NativeDispatchResult.Bool -> qjs_NewBool(ctx, if (r.v) 1 else 0)
        is NativeDispatchResult.Bytes -> r.v?.let { byteArrayToJsArray(ctx, it) } ?: jsNull()
        is NativeDispatchResult.AnyVal -> anyToJs(ctx, r.v)
        // Handle: 0 = null, 非 0 = 已注册对象 handle, JS 层按返回类型工厂包装
        is NativeDispatchResult.Handle -> if (r.v == 0L) jsNull() else qjs_NewFloat64(
            ctx,
            r.v.toDouble()
        )
        NativeDispatchResult.UNIT -> jsUndefined()
        NativeDispatchResult.NONE -> jsUndefined()
    }

    /** JS Array → Kotlin List<Any?> (递归取元素) */
    private fun jsArrayToList(ctx: CPointer<JSContext>, arr: CValue<JSValue>): List<Any?> {
        val result = mutableListOf<Any?>()
        memScoped {
            val pLen = alloc<LongVar>()
            if (JS_GetLength(ctx, arr, pLen.ptr) != 0) return@memScoped
            require(pLen.value in 0L..1000000L) { "Source argument array is too large" }
            val count = pLen.value.toInt()
            for (i in 0 until count) {
                val elem = JS_GetPropertyUint32(ctx, arr, i.toUInt())
                try {
                    result.add(fromJsValue(ctx, elem))
                } finally {
                    JS_FreeValue(ctx, elem)
                }
            }
        }
        return result
    }

    /** JS Array → ByteArray (每个元素 toByte) */
    private fun jsArrayToByteArray(ctx: CPointer<JSContext>, arr: CValue<JSValue>): ByteArray {
        val list = jsArrayToList(ctx, arr)
        return ByteArray(list.size) { (list[it] as? Number)?.toByte() ?: 0 }
    }

    /** ByteArray → JS Array (每个 byte 转 Int) */
    private fun byteArrayToJsArray(ctx: CPointer<JSContext>, bytes: ByteArray): CValue<JSValue> {
        val arr = JS_NewArray(ctx)
        bytes.forEachIndexed { i, b ->
            // JS_SetPropertyUint32 转移 jsV 所有权, 不需要 JS_FreeValue
            JS_SetPropertyUint32(ctx, arr, i.toUInt(), qjs_NewInt32(ctx, b.toInt()))
        }
        return arr
    }

    /** String → JSValue，按 UTF-16 单元传输。 */
    private fun stringToJsValue(ctx: CPointer<JSContext>, s: String?): CValue<JSValue> {
        if (s == null) return jsNull()
        return nativeJsString(ctx, s)
    }

    /** JSValue → String，按 UTF-16 单元传输。 */
    private fun jsValueToString(ctx: CPointer<JSContext>, v: CValue<JSValue>): String {
        return nativeStringOrNull(ctx, v).orEmpty()
    }

    /** JSValue → Kotlin 值 (基本类型 + Array + String; null/undefined → null, boolean → Boolean) */
    private fun fromJsValue(ctx: CPointer<JSContext>, v: CValue<JSValue>): Any? =
        NativeJsEngine.fromJsValue(ctx, v)

    /** 构造 JS undefined JSValue */
    private fun jsUndefined(): CValue<JSValue> = cValue {
        tag = JS_TAG_UNDEFINED.toLong()
    }

    /** 构造 JS null JSValue */
    private fun jsNull(): CValue<JSValue> = cValue {
        tag = JS_TAG_NULL.toLong()
    }

    /** List 取 String (index 越界或类型不符返回 "") */
    private fun List<Any?>.getString(index: Int): String =
        (getOrNull(index) as? String) ?: ""

    /**
     * java.get/head/post 的 headers 参数: JS 层已 JSON.stringify 成 JSON 文本 (对象或 JSON 字符串), 解析回 Map。
     * 对齐 JVM 端 dispatcher 的 Map 强转 (JS 对象 → Map) 与 JSON 字符串 → Map 两条路径。
     */
    private fun parseHeaderArg(raw: Any?): Map<String, String> {
        val json = raw as? String ?: return emptyMap()
        return GSON.fromJsonObject<Map<String, String>>(json).getOrNull() ?: emptyMap()
    }

    private fun List<Any?>.webDelay(index: Int): Long =
        (getOrNull(if (getOrNull(index) is Boolean) index + 1 else index) as? Number)?.toLong() ?: 0L

    /** List 取可空 String (webView html/url/js 等可空参数) */
    private fun List<Any?>.getStringOrNull(index: Int): String? =
        getOrNull(index) as? String

    /** data 参数: Utf8String 直传, JS Array → ByteArray, 其余 null (queryTTF 等可空数据参数)。 */
    private fun List<Any?>.getDataArgOrNull(index: Int): Any? = when (val v = getOrNull(index)) {
        is ByteArray -> v
        is String -> v
        is List<*> -> ByteArray(v.size) { (v[it] as? Number)?.toInt()?.toByte() ?: 0 }
        else -> null
    }

    /** 取 data 参数: Utf8String 直传, JS Array → ByteArray, 其余归一为空串 (与 getString 同宽松策略)。 */
    private fun List<Any?>.getDataArg(index: Int): Any = when (val v = getOrNull(index)) {
        is ByteArray -> v
        is String -> v
        is List<*> -> ByteArray(v.size) { (v[it] as? Number)?.toInt()?.toByte() ?: 0 }
        else -> ""
    }

    /** key/iv 参数转字节: Utf8String → UTF-8 字节, JS Array → ByteArray, null/undefined → null。 */
    private fun Any?.toBytesOrNull(): ByteArray? = when (this) {
        is ByteArray -> this
        is String -> encodeToByteArray()
        is List<*> -> ByteArray(size) { (this[it] as? Number)?.toInt()?.toByte() ?: 0 }
        else -> null
    }

    // ============ 规则引擎核心方法 (AnalyzeRuleCore / AnalyzeUrlCore 共用, [X4] 补齐) ============

    private fun Any.corePut(key: String, value: String): String = when (this) {
        is AnalyzeRuleCore -> put(key, value)
        is AnalyzeUrlCore -> put(key, value)
        else -> value
    }

    private fun Any.coreGet(key: String): String = when (this) {
        is AnalyzeRuleCore -> get(key)
        is AnalyzeUrlCore -> get(key)
        else -> ""
    }

    private fun Any.coreEvalJS(jsStr: String): Any? = when (this) {
        is AnalyzeRuleCore -> evalJS(jsStr)
        is AnalyzeUrlCore -> evalJS(jsStr)
        else -> null
    }

    /**
     * 任意 Kotlin 值 → JSValue (evalJS 返回值用):
     * 基本类型直转; Map/List 递归构造 JS 对象/数组 (JS_SetProperty 转移所有权, 不额外 free);
     * 其余对象 toString 降级 (与 JVM 反射包装的差异, 复杂对象不可桥接)。
     */
    private fun anyToJs(ctx: CPointer<JSContext>, value: Any?): CValue<JSValue> =
        NativeJsEngine.toJsValue(ctx, value) ?: stringToJsValue(ctx, value.toString())
}

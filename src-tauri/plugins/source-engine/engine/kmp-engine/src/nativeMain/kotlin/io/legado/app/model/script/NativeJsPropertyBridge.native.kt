// 源平台来源：data/src/nativeMain/kotlin/io/legado/app/model/script/NativeJsPropertyBridge.native.kt。此 actual 已复制进提取模块，构建不读取源平台目录。
@file:OptIn(kotlinx.cinterop.ExperimentalForeignApi::class)

package io.legado.app.model.script

import io.legado.app.data.entities.BaseBook
import io.legado.app.data.entities.BaseSource
import io.legado.app.data.entities.Book
import io.legado.app.data.entities.BookChapter
import io.legado.app.data.entities.BookChapterLike
import io.legado.app.data.entities.BookSource
import io.legado.app.help.source.SourceCacheProvider
import io.legado.app.help.source.SourceNetworkProvider
import io.legado.app.model.analyzeRule.AnalyzeRuleCore
import io.legado.app.model.analyzeRule.AnalyzeUrlCore
import io.legado.app.napi.quickjs.JSContext
import io.legado.app.napi.quickjs.JSValue
import io.legado.app.napi.quickjs.JS_NewArray
import io.legado.app.napi.quickjs.JS_NewObject
import io.legado.app.napi.quickjs.JS_SetPropertyStr
import io.legado.app.napi.quickjs.JS_SetPropertyUint32
import io.legado.app.napi.quickjs.JS_TAG_NULL
import io.legado.app.napi.quickjs.JS_TAG_UNDEFINED
import io.legado.app.napi.quickjs.qjs_NewBool
import io.legado.app.napi.quickjs.qjs_NewFloat64
import io.legado.app.napi.quickjs.qjs_NewInt32
import io.legado.app.utils.GSON
import io.legado.app.utils.toJson
import kotlinx.cinterop.CPointer
import kotlinx.cinterop.CValue
import kotlinx.cinterop.cValue

/**
 * native 端 (iOS/鸿蒙) 复杂对象属性桥: 手写静态属性分派表 (路线 C)。
 *
 * # 背景
 * [NativeJsEngine.toJsValue] 只桥接 JsExtensionsCommon, `book`/`chapter` 等 binding
 * 注入即 undefined; JVM 端这些属性靠 JavaObjectBridge 反射 findGetter 兜底
 * (Book/BookChapter 无 @JsApi 注解, KSP 分派表不覆盖), native 无反射, 只能手写静态表。
 *
 * # 方案
 * - propertyId 段 1700-2199 (避开 NativeJsExtensionsBridge 的 1-699 / 1000-1699):
 *   1700-1799 BaseBook (book) | 1800-1899 BookChapter (chapter) | 1900-1999 BookSource |
 *   2000-2099 BaseSource (source) | 2100-2199 AnalyzeUrlCore/AnalyzeRuleCore (java)
 * - 2200+ 带参分派 (经 [dispatchWithArgs], 由 NativeJsExtensionsBridge 解析参数后转发):
 *   2300-2399 BaseBook 变量/方法面 (getVariable/putVariable 等) | 2400-2499 BookChapter 变量/方法面 |
 *   2500-2599 cookie (SourceNetworkProvider) | 2600-2699 cache (SourceCacheProvider) |
 *   2700-3199 属性写 (setterId = getterId + 1000, 对应 1700-2199 的 var 字段) |
 *   DOM 方法由 JavaClassCompat 处理，与 Packages.org.jsoup 使用同一通路。
 *   注: 属性名→propId 在 JS 工厂静态绑定; 读/写经同一对象同一 propId 语义, setter 仅偏移 +1000,
 *   与 JVM 端 JavaObjectBridge 的 findGetter/findSetter (bean 属性读写) 对齐。
 * - JS 侧由工厂函数 (__createBookObj 等) 用 Object.defineProperty 挂 getter/setter,
 *   与 JsURL 属性段 (1500-1599) 同一模式; getter 调 `__nativeDispatch(handle, propId, [])`、
 *   setter 调 `__nativeDispatch(handle, propId + 1000, [v])` 回 Kotlin。
 * - 语义对齐 JVM 端 JsApiDispatcher.getProperty(target, name): miss 返回 null
 *   (调用方转 undefined, 等价 NO_MATCH); 写 miss 静默 (与 JVM 端 setter 反射找不到时一致)。
 * - 嵌套对象 (AnalyzeRuleCore.chapter/ruleData): getter 返回 handle, JS 侧工厂函数包装
 *   嵌套对象的 handle 统一登记到当前 scope，随本次执行或 scope 关闭回收。
 *
 * # hot path
 * 一次属性读取: 1 次 handle 表 HashMap get + 1 次 when(propId) 整数跳转 +
 * 字段直读 + 1 次 JSValue 构造。零堆分配 (bridge.dispatch 对 >= 1700 短路, 跳过参数数组解析)。
 */
const val PROPERTY_ID_BASE = 1700

/** 带参分派起始 (属性写/变量方法/cookie/cache, 经 [NativeJsPropertyBridge.dispatchWithArgs])。 */
const val PROPERTY_WRITE_BASE = 2200

object NativeJsPropertyBridge {

    /**
     * 属性分派: propId → 对象字段。未命中返回 null (调用方转 JS undefined)。
     *
     * 注: when(propId) 是整数跳转表 (table switch), 与 JVM 端反射 getter 相比
     * 无方法查找/无字符串比较; is 类型检查在工厂创建时已保证 (getter 只挂在对应类型对象上),
     * 此处 as? 仅为防御。
     */
    fun dispatch(ctx: CPointer<JSContext>, obj: Any, propId: Int): CValue<JSValue>? = when (propId) {
        // ============ BaseBook 属性 (1700-1799, book binding) ============
        1701 -> (obj as? BaseBook)?.let { stringToJsValue(ctx, it.bookUrl) } // bookUrl
        1702 -> (obj as? BaseBook)?.let { stringToJsValue(ctx, it.name) } // name
        1703 -> (obj as? BaseBook)?.let { stringToJsValue(ctx, it.author) } // author
        1704 -> (obj as? BaseBook)?.let { stringToJsValue(ctx, it.origin) } // origin
        1705 -> (obj as? BaseBook)?.let { stringToJsValue(ctx, it.originName) } // originName
        1706 -> (obj as? BaseBook)?.let { stringToJsValue(ctx, it.kind) } // kind
        1707 -> (obj as? BaseBook)?.let { stringToJsValue(ctx, it.wordCount) } // wordCount
        1708 -> (obj as? BaseBook)?.let { stringToJsValue(ctx, it.variable) } // variable
        1709 -> (obj as? BaseBook)?.let { stringToJsValue(ctx, it.infoHtml) } // infoHtml
        1710 -> (obj as? BaseBook)?.let { stringToJsValue(ctx, it.tocHtml) } // tocHtml
        1711 -> (obj as? BaseBook)?.let { qjs_NewInt32(ctx, it.type) } // type
        1712 -> (obj as? BaseBook)?.let { stringToJsValue(ctx, it.coverUrl) } // coverUrl
        1713 -> (obj as? BaseBook)?.let { stringToJsValue(ctx, it.intro) } // intro (注意不是 introduction)
        1714 -> (obj as? BaseBook)?.let { stringToJsValue(ctx, it.latestChapterTitle) } // latestChapterTitle
        1715 -> (obj as? BaseBook)?.let { stringToJsValue(ctx, it.tocUrl) } // tocUrl
        1716 -> (obj as? BaseBook)?.let { qjs_NewInt32(ctx, it.originOrder) } // originOrder

        // ============ Book 实体属性 (1717-1734, book binding) ============
        // 注: BaseBook 接口仅 15 个属性, Book 实体专属字段用 as? Book (与 JVM 反射实际类一致)
        1717 -> (obj as? Book)?.let { stringToJsValue(ctx, it.customTag) } // customTag
        1718 -> (obj as? Book)?.let { stringToJsValue(ctx, it.customCoverUrl) } // customCoverUrl
        1719 -> (obj as? Book)?.let { stringToJsValue(ctx, it.customIntro) } // customIntro
        1720 -> (obj as? Book)?.let { stringToJsValue(ctx, it.charset) } // charset
        1721 -> (obj as? Book)?.let { longToJsValue(ctx, it.group) } // group
        1722 -> (obj as? Book)?.let { longToJsValue(ctx, it.latestChapterTime) } // latestChapterTime
        1723 -> (obj as? Book)?.let { longToJsValue(ctx, it.lastCheckTime) } // lastCheckTime
        1724 -> (obj as? Book)?.let { qjs_NewInt32(ctx, it.lastCheckCount) } // lastCheckCount
        1725 -> (obj as? Book)?.let { qjs_NewInt32(ctx, it.totalChapterNum) } // totalChapterNum
        1726 -> (obj as? Book)?.let { stringToJsValue(ctx, it.durChapterTitle) } // durChapterTitle
        1727 -> (obj as? Book)?.let { qjs_NewInt32(ctx, it.durChapterIndex) } // durChapterIndex
        1728 -> (obj as? Book)?.let { qjs_NewInt32(ctx, it.durChapterPos) } // durChapterPos
        1729 -> (obj as? Book)?.let { longToJsValue(ctx, it.durChapterTime) } // durChapterTime
        1730 -> (obj as? Book)?.let { qjs_NewBool(ctx, if (it.canUpdate) 1 else 0) } // canUpdate
        1731 -> (obj as? Book)?.let { qjs_NewInt32(ctx, it.order) } // order
        1732 -> (obj as? Book)?.let { longToJsValue(ctx, it.syncTime) } // syncTime
        1733 -> (obj as? Book)?.let { stringListToJsArray(ctx, it.downloadUrls ?: emptyList()) } // downloadUrls
        1734 -> (obj as? Book)?.let { qjs_NewInt32(ctx, it.lastChapterIndex) } // lastChapterIndex (val, 只读)

        // ============ BookChapter 属性 (1800-1899, chapter binding) ============
        1801 -> (obj as? BookChapter)?.let { stringToJsValue(ctx, it.url) } // url
        1802 -> (obj as? BookChapterLike)?.let { stringToJsValue(ctx, it.title) } // title
        1803 -> (obj as? BookChapter)?.let { stringToJsValue(ctx, it.bookUrl) } // bookUrl
        1804 -> (obj as? BookChapter)?.let { qjs_NewInt32(ctx, it.index) } // index
        1805 -> (obj as? BookChapter)?.let { qjs_NewBool(ctx, if (it.isVolume) 1 else 0) } // isVolume
        1806 -> (obj as? BookChapter)?.let { qjs_NewBool(ctx, if (it.isVip) 1 else 0) } // isVip
        1807 -> (obj as? BookChapter)?.let { qjs_NewBool(ctx, if (it.isPay) 1 else 0) } // isPay
        1808 -> (obj as? BookChapter)?.let { stringToJsValue(ctx, it.resourceUrl) } // resourceUrl
        1809 -> (obj as? BookChapter)?.let { stringToJsValue(ctx, it.tag) } // tag
        1810 -> (obj as? BookChapter)?.let { stringToJsValue(ctx, it.wordCount) } // wordCount
        1811 -> (obj as? BookChapter)?.let { longToJsValue(ctx, it.start) } // start
        1812 -> (obj as? BookChapter)?.let { longToJsValue(ctx, it.end) } // end
        1813 -> (obj as? BookChapter)?.let { stringToJsValue(ctx, it.variable) } // variable
        1814 -> (obj as? BookChapter)?.let { stringToJsValue(ctx, it.startFragmentId) } // startFragmentId
        1815 -> (obj as? BookChapter)?.let { stringToJsValue(ctx, it.endFragmentId) } // endFragmentId
        1816 -> (obj as? BookChapter)?.let { stringToJsValue(ctx, it.titleMD5) } // titleMD5 (@Transient)

        // ============ BookSource 属性 (1900-1999, source binding 的 BookSource 面) ============
        1901 -> (obj as? BookSource)?.let { stringToJsValue(ctx, it.bookSourceUrl) } // bookSourceUrl
        1902 -> (obj as? BookSource)?.let { stringToJsValue(ctx, it.bookSourceName) } // bookSourceName
        1903 -> (obj as? BookSource)?.let { stringToJsValue(ctx, it.bookSourceGroup) } // bookSourceGroup
        1904 -> (obj as? BookSource)?.let { qjs_NewInt32(ctx, it.bookSourceType) } // bookSourceType
        1905 -> (obj as? BookSource)?.let { stringToJsValue(ctx, it.bookUrlPattern) } // bookUrlPattern
        1906 -> (obj as? BookSource)?.let { qjs_NewInt32(ctx, it.customOrder) } // customOrder
        1907 -> (obj as? BookSource)?.let { qjs_NewBool(ctx, if (it.enabled) 1 else 0) } // enabled
        1908 -> (obj as? BookSource)?.let { qjs_NewBool(ctx, if (it.enabledExplore) 1 else 0) } // enabledExplore

        // ============ BookSource 实体属性 (1909-1926, source binding 的 BookSource 面) ============
        1909 -> (obj as? BookSource)?.let { qjs_NewBool(ctx, if (it.enabledReview) 1 else 0) } // enabledReview
        1910 -> (obj as? BookSource)?.let { stringToJsValue(ctx, it.loginCheckJs) } // loginCheckJs
        1911 -> (obj as? BookSource)?.let { stringToJsValue(ctx, it.coverDecodeJs) } // coverDecodeJs
        1912 -> (obj as? BookSource)?.let { stringToJsValue(ctx, it.bookSourceComment) } // bookSourceComment
        1913 -> (obj as? BookSource)?.let { stringToJsValue(ctx, it.variableComment) } // variableComment
        1914 -> (obj as? BookSource)?.let { longToJsValue(ctx, it.lastUpdateTime) } // lastUpdateTime
        1915 -> (obj as? BookSource)?.let { longToJsValue(ctx, it.respondTime) } // respondTime
        1916 -> (obj as? BookSource)?.let { qjs_NewInt32(ctx, it.weight) } // weight
        1917 -> (obj as? BookSource)?.let { stringToJsValue(ctx, it.exploreUrl) } // exploreUrl
        1918 -> (obj as? BookSource)?.let { stringToJsValue(ctx, it.exploreScreen) } // exploreScreen
        1919 -> (obj as? BookSource)?.let { qjs_NewInt32(ctx, it.exploreStyle) } // exploreStyle
        1920 -> (obj as? BookSource)?.let { stringToJsValue(ctx, it.ruleExplore) } // ruleExplore
        1921 -> (obj as? BookSource)?.let { stringToJsValue(ctx, it.searchUrl) } // searchUrl
        1922 -> (obj as? BookSource)?.let { stringToJsValue(ctx, it.ruleSearch) } // ruleSearch
        1923 -> (obj as? BookSource)?.let { stringToJsValue(ctx, it.ruleBookInfo) } // ruleBookInfo
        1924 -> (obj as? BookSource)?.let { stringToJsValue(ctx, it.ruleToc) } // ruleToc
        1925 -> (obj as? BookSource)?.let { stringToJsValue(ctx, it.ruleContent) } // ruleContent
        1926 -> (obj as? BookSource)?.let { stringToJsValue(ctx, it.ruleReview) } // ruleReview

        // ============ BookSource 复杂对象属性 (搜索/详情/目录规则对象等, 嵌套对象无法桥接) ============
        // 1927 searchRule / 1928 exploreRule / 1929 bookInfoRule / 1930 tocRule / 1931 contentRule /
        // 1932 reviewRule: JVM 端反射返回 SearchRule 等嵌套对象, native 无嵌套对象桥, 保持 undefined
        // (两端不一致, 见报告"未覆盖: 嵌套复杂对象")
        // ============ BaseSource 属性 (2000-2099, source binding) ============
        2001 -> (obj as? BaseSource)?.let { stringToJsValue(ctx, it.concurrentRate) } // concurrentRate
        2002 -> (obj as? BaseSource)?.let { stringToJsValue(ctx, it.loginUrl) } // loginUrl
        2003 -> (obj as? BaseSource)?.let { stringToJsValue(ctx, it.loginUi) } // loginUi
        2004 -> (obj as? BaseSource)?.let { stringToJsValue(ctx, it.header) } // header
        2005 -> (obj as? BaseSource)?.let { boolOrNullToJsValue(ctx, it.enabledCookieJar) } // enabledCookieJar
        2006 -> (obj as? BaseSource)?.let { boolOrNullToJsValue(ctx, it.enableDangerousApi) } // enableDangerousApi
        2007 -> (obj as? BaseSource)?.let { stringToJsValue(ctx, it.jsLib) } // jsLib

        // ============ AnalyzeUrlCore 属性 (2100-2199, java binding) ============
        2101 -> (obj as? AnalyzeUrlCore)?.let { stringToJsValue(ctx, it.rawUrl) } // rawUrl
        2102 -> (obj as? AnalyzeUrlCore)?.let { stringToJsValue(ctx, it.urlNoQuery) } // urlNoQuery
        2103 -> (obj as? AnalyzeUrlCore)?.let { stringToJsValue(ctx, it.type) } // type
        2104 -> (obj as? AnalyzeUrlCore)?.let { longToJsValue(ctx, it.serverID) } // serverID
        2105 -> (obj as? AnalyzeUrlCore)?.let { stringToJsValue(ctx, GSON.toJson(it.headerMap)) } // headerMap → JSON (JS 侧 JSON.parse)
        2106 -> (obj as? AnalyzeUrlCore)?.let { stringToJsValue(ctx, it.url) } // url
        2107 -> (obj as? AnalyzeUrlCore)?.let { stringToJsValue(ctx, it.ruleUrl) } // ruleUrl
        2108 -> (obj as? AnalyzeUrlCore)?.let { stringToJsValue(ctx, it.encodedParams) } // encodedParams

        // ============ AnalyzeRuleCore 属性 (2109-2112, java binding 的 AnalyzeRule 面) ============
        // 对齐 JVM 端反射可见的 public var (chapter/nextChapterUrl/variables/ruleData);
        // chapter/ruleData 返回 handle, 由 JS 工厂包装为嵌套对象 (与 getSource 同模式)
        2109 -> (obj as? AnalyzeRuleCore)?.chapter?.let {
            qjs_NewFloat64(ctx, NativeJsExtensionsBridge.registerObject(it).toDouble())
        } // chapter (嵌套 handle)
        2110 -> (obj as? AnalyzeRuleCore)?.let { stringToJsValue(ctx, it.nextChapterUrl) } // nextChapterUrl
        2111 -> (obj as? AnalyzeRuleCore)?.variables?.let { anyToJs(ctx, it) } ?: jsNull() // variables (Map)
        2112 -> (obj as? AnalyzeRuleCore)?.ruleData?.let {
            qjs_NewFloat64(ctx, NativeJsExtensionsBridge.registerObject(it).toDouble())
        } // ruleData (嵌套 handle)

        else -> null // 未知属性 → undefined (等价 JsDispatchRegistry.NO_MATCH)
    }

    /**
     * 带参分派 (methodId >= [PROPERTY_WRITE_BASE], 由 NativeJsExtensionsBridge 解析参数后转发):
     *
     * - 2300-2399 BaseBook 变量/方法面: getVariable/putVariable/putBigVariable/getBigVariable /
     *   putCustomVariable/getCustomVariable/getKindList/getRealAuthor
     *   (对齐 JVM 端反射可调用的 RuleDataInterface 接口默认方法 + Book 方法)
     * - 2400-2499 BookChapter 变量/方法面: getVariable/putVariable/putBigVariable/getBigVariable /
     *   primaryStr/getFileName/getFontName
     * - 2500-2599 cookie: SourceNetworkProvider.getCookie/replaceCookie/removeCookie
     * - 2600-2699 cache: SourceCacheProvider.get/put/delete/getFromMemory/putMemory/deleteMemory /
     *   put(saveTime)/getInt/getLong/getDouble/getFloat/clearMemoryByPrefixes
     * - 2700-3199 属性写: setterId = getterId + 1000, 值转换宽松对齐 JVM 端 coerceValue
     *   (Number/String → Int/Long, 其余类型/null 不写, 与 setter 反射找不到时静默一致)
     *
     * @param args 已由调用方解析的 Kotlin 参数列表 (JS Array → List<Any?>, 基本类型直转)
     * @return 返回值 JSValue, 或 null (未命中/类型不符, 调用方转 undefined)
     */
    fun dispatchWithArgs(
        ctx: CPointer<JSContext>,
        obj: Any,
        methodId: Int,
        args: List<Any?>
    ): CValue<JSValue>? = when {
        // ============ BaseBook 变量/方法面 (2300-2399, book binding) ============
        methodId == 2301 -> (obj as? BaseBook)?.let {
            stringToJsValue(ctx, it.getVariable(args.getString(0)))
        } // getVariable(key)
        methodId == 2302 -> (obj as? BaseBook)?.let {
            qjs_NewBool(ctx, if (it.putVariable(args.getString(0), toNullableString(args.getOrNull(1)))) 1 else 0)
        } // putVariable(key, value) → Boolean
        methodId == 2303 -> (obj as? BaseBook)?.let {
            it.putBigVariable(args.getString(0), toNullableString(args.getOrNull(1)))
            jsUndefined()
        } // putBigVariable(key, value)
        methodId == 2304 -> (obj as? BaseBook)?.let {
            stringToJsValue(ctx, it.getBigVariable(args.getString(0)))
        } // getBigVariable(key) → String?
        methodId == 2305 -> (obj as? BaseBook)?.let {
            it.putCustomVariable(toNullableString(args.getOrNull(0)))
            jsUndefined()
        } // putCustomVariable(value)
        methodId == 2306 -> (obj as? BaseBook)?.let {
            stringToJsValue(ctx, it.getCustomVariable())
        } // getCustomVariable()
        methodId == 2307 -> (obj as? BaseBook)?.let {
            stringListToJsArray(ctx, it.getKindList())
        } // getKindList()
        methodId == 2308 -> (obj as? BaseBook)?.let {
            stringToJsValue(ctx, it.getRealAuthor())
        } // getRealAuthor()
        methodId == 2309 -> (obj as? Book)?.let {
            qjs_NewBool(ctx, if (it.getReverseToc()) 1 else 0)
        }
        methodId == 2310 -> (obj as? Book)?.let {
            it.setReverseToc(args.getOrNull(0) as? Boolean ?: false)
            jsUndefined()
        }

        // ============ BookChapter 变量/方法面 (2400-2499, chapter binding) ============
        methodId == 2401 -> (obj as? BookChapterLike)?.let {
            stringToJsValue(ctx, it.getVariable(args.getString(0)))
        } // getVariable(key)
        methodId == 2402 -> (obj as? BookChapterLike)?.let {
            qjs_NewBool(ctx, if (it.putVariable(args.getString(0), toNullableString(args.getOrNull(1)))) 1 else 0)
        } // putVariable(key, value) → Boolean
        methodId == 2403 -> (obj as? BookChapterLike)?.let {
            it.putBigVariable(args.getString(0), toNullableString(args.getOrNull(1)))
            jsUndefined()
        } // putBigVariable(key, value)
        methodId == 2404 -> (obj as? BookChapterLike)?.let {
            stringToJsValue(ctx, it.getBigVariable(args.getString(0)))
        } // getBigVariable(key) → String?
        methodId == 2405 -> (obj as? BookChapter)?.let { stringToJsValue(ctx, it.primaryStr()) } // primaryStr()
        methodId == 2406 -> (obj as? BookChapter)?.let { stringToJsValue(ctx, it.getFileName()) } // getFileName() (默认 nb)
        methodId == 2407 -> (obj as? BookChapter)?.let { stringToJsValue(ctx, it.getFontName()) } // getFontName()

        // ============ cookie (2500-2599, SourceNetworkProvider) ============
        methodId == 2501 -> (obj as? SourceNetworkProvider)?.let {
            stringToJsValue(ctx, it.getCookie(args.getString(0)))
        } // getCookie(tag)
        methodId == 2502 -> (obj as? SourceNetworkProvider)?.let {
            it.replaceCookie(args.getString(0), args.getString(1))
            jsUndefined()
        } // replaceCookie(tag, cookie)
        methodId == 2503 -> (obj as? SourceNetworkProvider)?.let {
            it.removeCookie(args.getString(0))
            jsUndefined()
        } // removeCookie(tag)

        // ============ cache (2600-2699, SourceCacheProvider) ============
        methodId == 2601 -> (obj as? SourceCacheProvider)?.let {
            stringToJsValue(ctx, it.get(args.getString(0)))
        } // get(key) → String?
        methodId == 2602 -> (obj as? SourceCacheProvider)?.let {
            it.put(args.getString(0), args.getString(1))
            jsUndefined()
        } // put(key, value)
        methodId == 2603 -> (obj as? SourceCacheProvider)?.let {
            it.delete(args.getString(0))
            jsUndefined()
        } // delete(key)
        methodId == 2604 -> (obj as? SourceCacheProvider)?.let {
            anyToJs(ctx, it.getFromMemory(args.getString(0)))
        } // getFromMemory(key) → Any?
        methodId == 2605 -> (obj as? SourceCacheProvider)?.let {
            args.getOrNull(1)?.let { v -> it.putMemory(args.getString(0), v) }
            jsUndefined()
        } // putMemory(key, value)
        methodId == 2606 -> (obj as? SourceCacheProvider)?.let {
            it.deleteMemory(args.getString(0))
            jsUndefined()
        } // deleteMemory(key)
        methodId == 2607 -> (obj as? SourceCacheProvider)?.let {
            it.put(args.getString(0), args.getString(1), (args.getOrNull(2) as? Number)?.toInt() ?: 0)
            jsUndefined()
        } // put(key, value, saveTime)
        methodId == 2608 -> (obj as? SourceCacheProvider)?.let {
            it.getInt(args.getString(0))?.let { v -> qjs_NewInt32(ctx, v) } ?: jsNull()
        } // getInt(key) → Int?
        methodId == 2609 -> (obj as? SourceCacheProvider)?.let {
            it.getLong(args.getString(0))?.let { v -> qjs_NewFloat64(ctx, v.toDouble()) } ?: jsNull()
        } // getLong(key) → Long?
        methodId == 2610 -> (obj as? SourceCacheProvider)?.let {
            it.getDouble(args.getString(0))?.let { v -> qjs_NewFloat64(ctx, v) } ?: jsNull()
        } // getDouble(key) → Double?
        methodId == 2611 -> (obj as? SourceCacheProvider)?.let {
            it.getFloat(args.getString(0))?.let { v -> qjs_NewFloat64(ctx, v.toDouble()) } ?: jsNull()
        } // getFloat(key) → Float?
        methodId == 2612 -> (obj as? SourceCacheProvider)?.let {
            val prefixes = (args.getOrNull(0) as? List<*>)
                ?.map { p -> p?.toString() ?: "" } ?: emptyList()
            it.clearMemoryByPrefixes(prefixes)
            jsUndefined()
        } // clearMemoryByPrefixes(prefixes)

        // ============ 属性写 (2700-3199, setterId = getterId + 1000) ============
        // 值转换: 非空字段 null/类型不符 → 跳过不写 (对齐 JVM 端 findSetter 找不到时静默);
        // 可空字段 → null 直写 (对齐 JVM 端 null 参数反射调用)。
        // ============ BaseBook 面 (2701-2716) ============
        methodId == 2701 -> (obj as? BaseBook)?.let { b ->
            toNullableString(args.getOrNull(0))?.let { b.bookUrl = it }
            jsUndefined()
        }
        methodId == 2702 -> (obj as? BaseBook)?.let { b ->
            toNullableString(args.getOrNull(0))?.let { b.name = it }
            jsUndefined()
        }
        methodId == 2703 -> (obj as? BaseBook)?.let { b ->
            toNullableString(args.getOrNull(0))?.let { b.author = it }
            jsUndefined()
        }
        methodId == 2704 -> (obj as? BaseBook)?.let { b ->
            toNullableString(args.getOrNull(0))?.let { b.origin = it }
            jsUndefined()
        }
        methodId == 2705 -> (obj as? BaseBook)?.let { b ->
            toNullableString(args.getOrNull(0))?.let { b.originName = it }
            jsUndefined()
        }
        methodId == 2706 -> (obj as? BaseBook)?.let { b -> b.kind = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2707 -> (obj as? BaseBook)?.let { b -> b.wordCount = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2708 -> (obj as? BaseBook)?.let { b -> b.variable = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2709 -> (obj as? BaseBook)?.let { b -> b.infoHtml = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2710 -> (obj as? BaseBook)?.let { b -> b.tocHtml = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2711 -> (obj as? BaseBook)?.let { b ->
            toIntOrNull(args.getOrNull(0))?.let { b.type = it }
            jsUndefined()
        }
        methodId == 2712 -> (obj as? BaseBook)?.let { b -> b.coverUrl = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2713 -> (obj as? BaseBook)?.let { b -> b.intro = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2714 -> (obj as? BaseBook)?.let { b -> b.latestChapterTitle = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2715 -> (obj as? BaseBook)?.let { b ->
            toNullableString(args.getOrNull(0))?.let { b.tocUrl = it }
            jsUndefined()
        }
        methodId == 2716 -> (obj as? BaseBook)?.let { b ->
            toIntOrNull(args.getOrNull(0))?.let { b.originOrder = it }
            jsUndefined()
        }

        // ============ Book 实体面 (2717-2733) ============
        methodId == 2717 -> (obj as? Book)?.let { b -> b.customTag = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2718 -> (obj as? Book)?.let { b -> b.customCoverUrl = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2719 -> (obj as? Book)?.let { b -> b.customIntro = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2720 -> (obj as? Book)?.let { b -> b.charset = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2721 -> (obj as? Book)?.let { b ->
            toLongOrNull(args.getOrNull(0))?.let { b.group = it }
            jsUndefined()
        }
        methodId == 2722 -> (obj as? Book)?.let { b ->
            toLongOrNull(args.getOrNull(0))?.let { b.latestChapterTime = it }
            jsUndefined()
        }
        methodId == 2723 -> (obj as? Book)?.let { b ->
            toLongOrNull(args.getOrNull(0))?.let { b.lastCheckTime = it }
            jsUndefined()
        }
        methodId == 2724 -> (obj as? Book)?.let { b ->
            toIntOrNull(args.getOrNull(0))?.let { b.lastCheckCount = it }
            jsUndefined()
        }
        methodId == 2725 -> (obj as? Book)?.let { b ->
            toIntOrNull(args.getOrNull(0))?.let { b.totalChapterNum = it }
            jsUndefined()
        }
        methodId == 2726 -> (obj as? Book)?.let { b -> b.durChapterTitle = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2727 -> (obj as? Book)?.let { b ->
            toIntOrNull(args.getOrNull(0))?.let { b.durChapterIndex = it }
            jsUndefined()
        }
        methodId == 2728 -> (obj as? Book)?.let { b ->
            toIntOrNull(args.getOrNull(0))?.let { b.durChapterPos = it }
            jsUndefined()
        }
        methodId == 2729 -> (obj as? Book)?.let { b ->
            toLongOrNull(args.getOrNull(0))?.let { b.durChapterTime = it }
            jsUndefined()
        }
        methodId == 2730 -> (obj as? Book)?.let { b ->
            (args.getOrNull(0) as? Boolean)?.let { b.canUpdate = it }
            jsUndefined()
        }
        methodId == 2731 -> (obj as? Book)?.let { b ->
            toIntOrNull(args.getOrNull(0))?.let { b.order = it }
            jsUndefined()
        }
        methodId == 2732 -> (obj as? Book)?.let { b ->
            toLongOrNull(args.getOrNull(0))?.let { b.syncTime = it }
            jsUndefined()
        }
        methodId == 2733 -> (obj as? Book)?.let { b ->
            (args.getOrNull(0) as? List<*>)?.let { list ->
                b.downloadUrls = list.map { it?.toString() ?: "" }
            }
            jsUndefined()
        }
        // 2734 lastChapterIndex: val 只读, 无 setter (两端一致)

        // ============ BookChapter 面 (2801-2816) ============
        methodId == 2801 -> (obj as? BookChapter)?.let { b ->
            toNullableString(args.getOrNull(0))?.let { b.url = it }
            jsUndefined()
        }
        methodId == 2802 -> (obj as? BookChapter)?.let { b ->
            toNullableString(args.getOrNull(0))?.let { b.title = it }
            jsUndefined()
        }
        methodId == 2803 -> (obj as? BookChapter)?.let { b ->
            toNullableString(args.getOrNull(0))?.let { b.bookUrl = it }
            jsUndefined()
        }
        methodId == 2804 -> (obj as? BookChapter)?.let { b ->
            toIntOrNull(args.getOrNull(0))?.let { b.index = it }
            jsUndefined()
        }
        methodId == 2805 -> (obj as? BookChapter)?.let { b ->
            (args.getOrNull(0) as? Boolean)?.let { b.isVolume = it }
            jsUndefined()
        }
        methodId == 2806 -> (obj as? BookChapter)?.let { b ->
            (args.getOrNull(0) as? Boolean)?.let { b.isVip = it }
            jsUndefined()
        }
        methodId == 2807 -> (obj as? BookChapter)?.let { b ->
            (args.getOrNull(0) as? Boolean)?.let { b.isPay = it }
            jsUndefined()
        }
        methodId == 2808 -> (obj as? BookChapter)?.let { b -> b.resourceUrl = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2809 -> (obj as? BookChapter)?.let { b -> b.tag = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2810 -> (obj as? BookChapter)?.let { b -> b.wordCount = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2811 -> (obj as? BookChapter)?.let { b -> b.start = toLongOrNull(args.getOrNull(0)); jsUndefined() }
        methodId == 2812 -> (obj as? BookChapter)?.let { b -> b.end = toLongOrNull(args.getOrNull(0)); jsUndefined() }
        methodId == 2813 -> (obj as? BookChapter)?.let { b -> b.variable = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2814 -> (obj as? BookChapter)?.let { b -> b.startFragmentId = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2815 -> (obj as? BookChapter)?.let { b -> b.endFragmentId = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2816 -> (obj as? BookChapter)?.let { b -> b.titleMD5 = toNullableString(args.getOrNull(0)); jsUndefined() }

        // ============ BookSource 面 (2901-2926) ============
        methodId == 2901 -> (obj as? BookSource)?.let { b ->
            toNullableString(args.getOrNull(0))?.let { b.bookSourceUrl = it }
            jsUndefined()
        }
        methodId == 2902 -> (obj as? BookSource)?.let { b ->
            toNullableString(args.getOrNull(0))?.let { b.bookSourceName = it }
            jsUndefined()
        }
        methodId == 2903 -> (obj as? BookSource)?.let { b -> b.bookSourceGroup = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2904 -> (obj as? BookSource)?.let { b ->
            toIntOrNull(args.getOrNull(0))?.let { b.bookSourceType = it }
            jsUndefined()
        }
        methodId == 2905 -> (obj as? BookSource)?.let { b -> b.bookUrlPattern = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2906 -> (obj as? BookSource)?.let { b ->
            toIntOrNull(args.getOrNull(0))?.let { b.customOrder = it }
            jsUndefined()
        }
        methodId == 2907 -> (obj as? BookSource)?.let { b ->
            (args.getOrNull(0) as? Boolean)?.let { b.enabled = it }
            jsUndefined()
        }
        methodId == 2908 -> (obj as? BookSource)?.let { b ->
            (args.getOrNull(0) as? Boolean)?.let { b.enabledExplore = it }
            jsUndefined()
        }
        methodId == 2909 -> (obj as? BookSource)?.let { b ->
            (args.getOrNull(0) as? Boolean)?.let { b.enabledReview = it }
            jsUndefined()
        }
        methodId == 2910 -> (obj as? BookSource)?.let { b -> b.loginCheckJs = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2911 -> (obj as? BookSource)?.let { b -> b.coverDecodeJs = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2912 -> (obj as? BookSource)?.let { b -> b.bookSourceComment = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2913 -> (obj as? BookSource)?.let { b -> b.variableComment = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2914 -> (obj as? BookSource)?.let { b ->
            toLongOrNull(args.getOrNull(0))?.let { b.lastUpdateTime = it }
            jsUndefined()
        }
        methodId == 2915 -> (obj as? BookSource)?.let { b ->
            toLongOrNull(args.getOrNull(0))?.let { b.respondTime = it }
            jsUndefined()
        }
        methodId == 2916 -> (obj as? BookSource)?.let { b ->
            toIntOrNull(args.getOrNull(0))?.let { b.weight = it }
            jsUndefined()
        }
        methodId == 2917 -> (obj as? BookSource)?.let { b -> b.exploreUrl = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2918 -> (obj as? BookSource)?.let { b -> b.exploreScreen = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2919 -> (obj as? BookSource)?.let { b ->
            toIntOrNull(args.getOrNull(0))?.let { b.exploreStyle = it }
            jsUndefined()
        }
        methodId == 2920 -> (obj as? BookSource)?.let { b -> b.ruleExplore = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2921 -> (obj as? BookSource)?.let { b -> b.searchUrl = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2922 -> (obj as? BookSource)?.let { b -> b.ruleSearch = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2923 -> (obj as? BookSource)?.let { b -> b.ruleBookInfo = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2924 -> (obj as? BookSource)?.let { b -> b.ruleToc = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2925 -> (obj as? BookSource)?.let { b -> b.ruleContent = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 2926 -> (obj as? BookSource)?.let { b -> b.ruleReview = toNullableString(args.getOrNull(0)); jsUndefined() }

        // ============ BaseSource 面 (3001-3007) ============
        methodId == 3001 -> (obj as? BaseSource)?.let { b -> b.concurrentRate = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 3002 -> (obj as? BaseSource)?.let { b -> b.loginUrl = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 3003 -> (obj as? BaseSource)?.let { b -> b.loginUi = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 3004 -> (obj as? BaseSource)?.let { b -> b.header = toNullableString(args.getOrNull(0)); jsUndefined() }
        methodId == 3005 -> (obj as? BaseSource)?.let { b -> b.enabledCookieJar = args.getOrNull(0) as? Boolean; jsUndefined() }
        methodId == 3006 -> jsUndefined() // enableDangerousApi 对 JS 只读 (dangerousApi 授权只能来自书源 JSON/编辑页)
        methodId == 3007 -> (obj as? BaseSource)?.let { b -> b.jsLib = toNullableString(args.getOrNull(0)); jsUndefined() }

        // ============ AnalyzeUrlCore 面 (3102/3106/3108) ============
        // 3101 rawUrl(val) / 3103 type(val get) / 3104 serverID(val get) / 3105 headerMap(val) 只读,
        // 3107 ruleUrl(protected set) 两端均不可写 (JVM getMethods 不含 protected setter)
        methodId == 3102 -> (obj as? AnalyzeUrlCore)?.let { b ->
            toNullableString(args.getOrNull(0))?.let { b.urlNoQuery = it }
            jsUndefined()
        }
        methodId == 3106 -> (obj as? AnalyzeUrlCore)?.let { b ->
            toNullableString(args.getOrNull(0))?.let { b.url = it }
            jsUndefined()
        }
        methodId == 3108 -> (obj as? AnalyzeUrlCore)?.let { b -> b.encodedParams = toNullableString(args.getOrNull(0)); jsUndefined() }

        // ============ AnalyzeRuleCore 面 (3110) ============
        // 3109 chapter / 3112 ruleData (嵌套对象, JS 参数面无法还原) / 3111 variables
        // (JS plain object 参数无法还原为 Kotlin Map) 不写, 报告"未覆盖"
        methodId == 3110 -> (obj as? AnalyzeRuleCore)?.let { b -> b.nextChapterUrl = toNullableString(args.getOrNull(0)); jsUndefined() }

        else -> null // 未命中 → undefined (等价 JsDispatchRegistry.NO_MATCH)
    }

    // ============ 工具方法: JSValue 构造 ============

    private fun stringToJsValue(ctx: CPointer<JSContext>, s: String?): CValue<JSValue> =
        if (s == null) jsNull() else nativeJsString(ctx, s)

    private fun boolOrNullToJsValue(ctx: CPointer<JSContext>, b: Boolean?): CValue<JSValue> =
        b?.let { qjs_NewBool(ctx, if (it) 1 else 0) } ?: jsNull()

    private fun longToJsValue(ctx: CPointer<JSContext>, v: Long?): CValue<JSValue> =
        v?.let { qjs_NewFloat64(ctx, it.toDouble()) } ?: jsNull()

    private fun jsNull(): CValue<JSValue> = cValue {
        tag = JS_TAG_NULL.toLong()
    }

    /** 构造 JS undefined JSValue (属性写/无返回值方法的返回值)。 */
    private fun jsUndefined(): CValue<JSValue> = cValue {
        tag = JS_TAG_UNDEFINED.toLong()
    }

    // ============ 工具方法: setter 值转换 (宽松对齐 JVM 端 coerceValue) ============

    /** null 保持 null; String 原样; 其余 toString (JS Number/Boolean → 字符串, 对齐 rhino coerce)。 */
    private fun toNullableString(v: Any?): String? = when (v) {
        null -> null
        is String -> v
        else -> v.toString()
    }

    /** Number 直转; String 解析; 其余 null (调用方对 null 跳过不写)。 */
    private fun toIntOrNull(v: Any?): Int? = when (v) {
        null -> null
        is Number -> v.toInt()
        is String -> v.toIntOrNull()
        else -> null
    }

    /** Number 直转; String 解析; 其余 null。 */
    private fun toLongOrNull(v: Any?): Long? = when (v) {
        null -> null
        is Number -> v.toLong()
        is String -> v.toLongOrNull()
        else -> null
    }

    // ============ 工具方法: JSValue 构造 (数组/对象) ============

    /** List<String> → JS Array (downloadUrls 等)。 */
    private fun stringListToJsArray(ctx: CPointer<JSContext>, list: List<String>): CValue<JSValue> {
        val arr = JS_NewArray(ctx)
        list.forEachIndexed { i, s ->
            JS_SetPropertyUint32(ctx, arr, i.toUInt(), nativeJsString(ctx, s))
        }
        return arr
    }

    /** List<Any?> 取 String (越界或类型不符返回 ""), 与 NativeJsExtensionsBridge.getString 同语义。 */
    private fun List<Any?>.getString(index: Int): String =
        (getOrNull(index) as? String) ?: ""

    /** 任意 Kotlin 值 → JSValue (variables 属性/getFromMemory 返回值用, 与 bridge 的 anyToJs 同语义)。 */
    private fun anyToJs(ctx: CPointer<JSContext>, value: Any?): CValue<JSValue> =
        NativeJsEngine.toJsValue(ctx, value) ?: stringToJsValue(ctx, value.toString())

}

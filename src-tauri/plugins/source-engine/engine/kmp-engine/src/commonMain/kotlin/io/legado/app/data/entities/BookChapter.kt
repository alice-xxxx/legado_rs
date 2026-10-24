package io.legado.app.data.entities

import io.legado.app.help.RuleBigDataProviders
import io.legado.app.model.analyzeRule.AnalyzeUrlCore
import io.legado.app.model.analyzeRule.RuleDataInterface
import io.legado.app.utils.MD5Utils
import io.legado.app.utils.NetworkUtils
import io.legado.app.utils.decodeStringMapOrNull
import io.legado.app.utils.encodeStringMap
import io.legado.app.utils.isDataUrl
import kotlinx.serialization.Serializable
import kotlinx.serialization.Transient

@Serializable
    // 删除书籍时自动删除章节
data class BookChapter(
    var url: String = "",               // 章节地址
    override var title: String = "",             // 章节标题
    var isVolume: Boolean = false,      // 是否是卷名
    var bookUrl: String = "",           // 书籍地址
    var index: Int = 0,                 // 章节序号
    var isVip: Boolean = false,         // 是否VIP
    var isPay: Boolean = false,         // 是否已购买
    var resourceUrl: String? = null,    // 音频/视频真实资源 URL
    var tag: String? = null,            // 更新时间或其他章节附加信息
    var wordCount: String? = null,      // 本章节字数
    var start: Long? = null,            // 章节起始位置
    var end: Long? = null,              // 章节终止位置
    var startFragmentId: String? = null,  //EPUB书籍当前章节的fragmentId
    var endFragmentId: String? = null,    //EPUB书籍下一章节的fragmentId
    var variable: String? = null        //变量
) : RuleDataInterface, BookChapterLike {

    override val variableMap: HashMap<String, String> by lazy {
        decodeStringMapOrNull(variable) ?: hashMapOf()
    }

    @Transient
    var titleMD5: String? = null

    override fun putVariable(key: String, value: String?): Boolean {
        if (super<RuleDataInterface>.putVariable(key, value)) {
            variable = encodeStringMap(variableMap)
        }
        return true
    }

    override fun putBigVariable(key: String, value: String?) {
        RuleBigDataProviders.impl?.putChapterVariable(bookUrl, url, key, value)
    }

    override fun getBigVariable(key: String): String? {
        return RuleBigDataProviders.impl?.getChapterVariable(bookUrl, url, key)
    }

    // 不覆写 equals/hashCode: 只比 url 会让 List<BookChapter> 整表判等, 目录刷新后
    // 标题/VIP 变而 url 不变时 StateFlow 吞掉更新; 去重/定位处已显式改为按 url 比较

    fun primaryStr(): String {
        return bookUrl + url
    }

    private fun ensureTitleMD5Init() {
        if (titleMD5 == null) {
            titleMD5 = MD5Utils.md5Encode16(title)
        }
    }

    fun getFileName(suffix: String = "nb"): String {
        ensureTitleMD5Init()
        return "${index.toString().padStart(5, '0')}-$titleMD5.$suffix"
    }

    @Suppress("unused")
    fun getFontName(): String {
        ensureTitleMD5Init()
        return "${index.toString().padStart(5, '0')}-$titleMD5.ttf"
    }

    fun getAbsoluteURL(book: Book): String {
        //二级目录解析的卷链接为空 返回目录页的链接
        if (url.startsWith(title) && isVolume) return book.tocUrl
        if (url.isDataUrl()) return url
        // Pattern.matcher → Regex.find: match.range.first 对应 matcher.start(), match.range.last + 1 对应 matcher.end()
        val urlMatch = AnalyzeUrlCore.paramPattern.find(url)
        val urlBefore = urlMatch?.let { url.substring(0, it.range.first) } ?: url
        val urlAbsoluteBefore = NetworkUtils.getAbsoluteURL(book.tocUrl, urlBefore)
        return if (urlBefore.length == url.length) {
            urlAbsoluteBefore
        } else {
            // urlMatch 非 null (urlBefore.length != url.length 意味着 find() 匹配成功)
            "$urlAbsoluteBefore," + url.substring(urlMatch!!.range.last + 1)
        }
    }
}

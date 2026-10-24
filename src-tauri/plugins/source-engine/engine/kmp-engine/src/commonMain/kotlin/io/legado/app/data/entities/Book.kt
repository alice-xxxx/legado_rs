package io.legado.app.data.entities

import io.legado.app.constant.BookType
import io.legado.app.help.book.getFolderNameNoCache
import io.legado.app.utils.decodeStringMapOrNull
import io.legado.app.utils.systemCurrentTimeMillis
import kotlinx.serialization.KSerializer
import kotlinx.serialization.Serializable
import kotlinx.serialization.Transient
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import kotlinx.serialization.json.Json

// LocalDate 定义在同 package 的 LocalDate.kt (纯 Kotlin 公历日期, 不引 java.time)

@Serializable

data class Book(
    // 详情页Url(本地书源存储完整文件路径)

    override var bookUrl: String = "",
    // 目录页Url (toc=table of Contents)

    override var tocUrl: String = "",
    // 书源URL(默认BookType.local)

    override var origin: String = BookType.localTag,
    //书源名称 or 本地书籍文件名

    override var originName: String = "",
    // 书籍名称(书源获取)

    override var name: String = "",
    // 作者名称(书源获取)

    override var author: String = "",
    // 分类信息(书源获取)
    override var kind: String? = null,
    // 分类信息(用户修改)
    var customTag: String? = null,
    // 封面Url(书源获取)
    override var coverUrl: String? = null,
    // 封面Url(用户修改)
    var customCoverUrl: String? = null,
    // 简介内容(书源获取)
    override var intro: String? = null,
    // 简介内容(用户修改)
    var customIntro: String? = null,
    // 自定义字符集名称(仅适用于本地书籍)
    var charset: String? = null,
    // 类型,详见BookType

    override var type: Int = BookType.text,
    // 自定义分组索引号

    var group: Long = 0,
    // 最新章节标题
    override var latestChapterTitle: String? = null,
    // 最新章节标题更新时间

    var latestChapterTime: Long = systemCurrentTimeMillis(),
    // 最近一次更新书籍信息的时间

    var lastCheckTime: Long = systemCurrentTimeMillis(),
    // 最近一次发现新章节的数量

    var lastCheckCount: Int = 0,
    // 书籍目录总数

    var totalChapterNum: Int = 0,
    // 当前章节名称
    var durChapterTitle: String? = null,
    // 当前章节索引

    var durChapterIndex: Int = 0,
    // 当前阅读的进度(首行字符的索引位置)

    var durChapterPos: Int = 0,
    // 最近一次阅读书籍的时间(打开正文的时间)

    var durChapterTime: Long = systemCurrentTimeMillis(),
    //字数
    override var wordCount: String? = null,
    // 刷新书架时更新书籍信息

    var canUpdate: Boolean = true,
    // 手动排序

    var order: Int = 0,
    //书源排序

    override var originOrder: Int = 0,
    // 自定义书籍变量信息(用于书源规则检索书籍信息)
    override var variable: String? = null,
    //阅读设置
    var readConfig: ReadConfig? = null,
    //同步时间

    var syncTime: Long = 0L
) : BaseBook {

    override val variableMap: HashMap<String, String> by lazy {
        decodeStringMapOrNull(variable) ?: hashMapOf()
    }

    @Transient
    override var infoHtml: String? = null

    @Transient
    override var tocHtml: String? = null

    @Transient
    var downloadUrls: List<String>? = null

    @Transient
    private var folderName: String? = null

    val lastChapterIndex get() = totalChapterNum - 1

    fun getDisplayCover() = getDisplayCoverRef()

    fun getDisplayCoverRef() = customCoverUrl.takeUnless { it.isNullOrEmpty() } ?: coverUrl

    fun getDisplayIntro() = customIntro.takeUnless { it.isNullOrEmpty() } ?: intro

    val config: ReadConfig
        get() = readConfig ?: ReadConfig().also { readConfig = it }

    // Keep the source-script API while the value lives in ReadConfig.
    fun getReverseToc(): Boolean = config.reverseToc

    fun setReverseToc(reverse: Boolean) {
        config.reverseToc = reverse
    }

    fun getStartDate(): LocalDate? {
        if (!config.readSimulating || config.startDate == null) {
            return localDateNow()
        }
        return config.startDate
    }

    fun getStartChapter(): Int {
        if (config.readSimulating) return config.startChapter ?: 0
        return this.durChapterIndex
    }
    fun getFolderName(): String {
        folderName?.let {
            return it
        }
        //防止书名过长,只取9位
        folderName = getFolderNameNoCache()
        return folderName!!
    }

    fun toSearchBook() = SearchBook(
        name = name,
        author = author,
        kind = kind,
        bookUrl = bookUrl,
        origin = origin,
        originName = originName,
        type = type,
        wordCount = wordCount,
        latestChapterTitle = latestChapterTitle,
        coverUrl = coverUrl,
        intro = intro,
        tocUrl = tocUrl,
        originOrder = originOrder,
        variable = variable
    ).apply {
        this.infoHtml = this@Book.infoHtml
        this.tocHtml = this@Book.tocHtml
    }

    @Suppress("ConstPropertyName")
    companion object {
        const val hTag = 2L
        const val rubyTag = 4L
        const val imgStyleDefault = "DEFAULT"
        const val imgStyleFull = "FULL"
        const val imgStyleText = "TEXT"
        const val imgStyleSingle = "SINGLE"
    }

    @Serializable
    data class ReadConfig(
        var reverseToc: Boolean = false,
        //var pageAnim: Int? = null,
        var reSegment: Boolean = false,
        var imageStyle: String? = null,
        var useReplaceRule: Boolean? = null,// 正文使用净化替换规则
        var delTag: Long = 0L,//去除标签
        var ttsEngine: String? = null,
        var splitLongChapter: Boolean = true,
        var readSimulating: Boolean = false,
        @Serializable(with = LocalDateAsGsonSerializer::class)
        var startDate: LocalDate? = null,
        var startChapter: Int? = null,     // 用户设置的起始章节
        var dailyChapters: Int = 3    // 用户设置的每日更新章节数
    )

    object LocalDateAsGsonSerializer : KSerializer<LocalDate> {
        @Serializable
        private data class Surrogate(val year: Int, val month: Int, val day: Int)

        override val descriptor: SerialDescriptor = Surrogate.serializer().descriptor

        override fun serialize(encoder: Encoder, value: LocalDate) {
            val (year, month, day) = value.toYearMonthDay()
            encoder.encodeSerializableValue(
                Surrogate.serializer(),
                Surrogate(year, month, day)
            )
        }

        override fun deserialize(decoder: Decoder): LocalDate {
            val s = decoder.decodeSerializableValue(Surrogate.serializer())
            return localDateOf(s.year, s.month, s.day)
        }
    }

    class Converters {

        fun readConfigToString(config: ReadConfig?): String? {
            if (config == null || config == ReadConfig()) return null
            return readConfigJson.encodeToString(config)
        }

        fun stringToReadConfig(json: String?): ReadConfig? {
            json ?: return null
            return kotlin.runCatching {
                readConfigJson.decodeFromString<ReadConfig>(json)
            }.getOrNull()
        }

        companion object {

            private val readConfigJson = Json {
                ignoreUnknownKeys = true
            }
        }
    }
}

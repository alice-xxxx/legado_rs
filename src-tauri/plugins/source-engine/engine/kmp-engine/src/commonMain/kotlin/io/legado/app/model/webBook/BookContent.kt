package io.legado.app.model.webBook

import io.legado.app.constant.AppConst
import io.legado.app.constant.AppPattern
import io.legado.app.data.entities.Book
import io.legado.app.data.entities.BookChapter
import io.legado.app.data.entities.BookSource
import io.legado.app.data.entities.rule.ContentRule
import io.legado.app.exception.ContentEmptyException
import io.legado.app.exception.NoStackTraceException
import io.legado.app.help.book.isAudio
import io.legado.app.help.book.isRss
import io.legado.app.help.book.isVideo
import io.legado.app.help.i18n.AppStringKey
import io.legado.app.help.i18n.appString
import io.legado.app.help.source.SourceDebugLoggers
import io.legado.app.model.analyzeRule.AnalyzeRuleFactories
import io.legado.app.model.analyzeRule.AnalyzeUrlFactories
import io.legado.app.utils.HtmlFormatter
import io.legado.app.utils.NetworkUtils
import io.legado.app.utils.mapAsync
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.flow

object BookContent {

    @Throws(Exception::class)
    suspend fun analyzeContent(
        bookSource: BookSource,
        book: Book,
        bookChapter: BookChapter,
        baseUrl: String,
        redirectUrl: String,
        body: String?,
        nextChapterUrl: String?
    ): String {
        body ?: throw NoStackTraceException(
            appString(AppStringKey.error_get_web_content, baseUrl)
        )
        SourceDebugLoggers.impl?.log(bookSource.bookSourceUrl, "≡获取成功:${baseUrl}")
        SourceDebugLoggers.impl?.log(bookSource.bookSourceUrl, body, state = 40, showTime = false)
        val contentList = arrayListOf<String>()
        val nextUrlList = arrayListOf(redirectUrl)
        val contentRule = bookSource.contentRule
        val analyzeRule = AnalyzeRuleFactories.create(book, bookSource)
        analyzeRule.setContent(body, baseUrl)
        analyzeRule.setRedirectUrl(redirectUrl)
        analyzeRule.coroutineContext = currentCoroutineContext()
        analyzeRule.chapter = bookChapter
        analyzeRule.nextChapterUrl = nextChapterUrl
        currentCoroutineContext().ensureActive()
        val titleRule = contentRule.title
        if (!titleRule.isNullOrBlank()) {
            val title = analyzeRule.runCatching {
                getString(titleRule)
            }.onFailure {
                SourceDebugLoggers.impl?.log(bookSource.bookSourceUrl, "获取标题出错, ${it.message}")
            }.getOrNull()
            if (!title.isNullOrBlank()) {
                bookChapter.title = title
                bookChapter.titleMD5 = null
            }
        }
        var contentData = analyzeContent(
            book, baseUrl, body, contentRule, bookChapter, bookSource, nextChapterUrl,
            redirectUrl = redirectUrl
        )
        contentList.add(contentData.first)
        if (contentData.second.isNotEmpty() && nextChapterUrl == null && bookChapter.index < book.totalChapterNum - 1) throw Exception(
            "未传入下一章节URL"
        )

        if (contentData.second.size == 1) {
            var nextUrl = contentData.second[0]
            while (nextUrl.isNotEmpty() && !nextUrlList.contains(nextUrl)) {
                if (!nextChapterUrl.isNullOrEmpty() && NetworkUtils.getAbsoluteURL(
                        redirectUrl, nextUrl
                    ) == NetworkUtils.getAbsoluteURL(redirectUrl, nextChapterUrl)
                ) break
                nextUrlList.add(nextUrl)
                currentCoroutineContext().ensureActive()
                val analyzeUrl = AnalyzeUrlFactories.create(
                    rawUrl = nextUrl,
                    source = bookSource,
                    ruleData = book,
                    coroutineContext = currentCoroutineContext()
                )
                val res = analyzeUrl.getStrResponseAwait() //控制并发访问
                res.body?.let { nextBody ->
                    contentData = analyzeContent(
                        book,
                        nextUrl,
                        nextBody,
                        contentRule,
                        bookChapter,
                        bookSource,
                        nextChapterUrl,
                        printLog = false,
                        redirectUrl = res.url
                    )
                    nextUrl = if (contentData.second.isNotEmpty()) contentData.second[0] else ""
                    contentList.add(contentData.first)
                    SourceDebugLoggers.impl?.log(bookSource.bookSourceUrl, "第${contentList.size}页完成")
                }
            }
            SourceDebugLoggers.impl?.log(bookSource.bookSourceUrl, "◇本章总页数:${nextUrlList.size}")
        } else if (contentData.second.size > 1) {
            SourceDebugLoggers.impl?.log(bookSource.bookSourceUrl, "◇并发解析正文,总页数:${contentData.second.size}")
            flow {
                for (urlStr in contentData.second) {
                    emit(urlStr)
                }
            }.mapAsync(minOf(16, AppConst.MAX_THREAD)) { urlStr ->
                val analyzeUrl = AnalyzeUrlFactories.create(
                    rawUrl = urlStr,
                    source = bookSource,
                    ruleData = book,
                    coroutineContext = currentCoroutineContext()
                )
                val res = analyzeUrl.getStrResponseAwait() //控制并发访问
                analyzeContent(
                    book,
                    urlStr,
                    res.body!!,
                    contentRule,
                    bookChapter,
                    bookSource,
                    nextChapterUrl,
                    getNextPageUrl = false,
                    printLog = false,
                    redirectUrl = res.url
                ).first
            }.collect {
                currentCoroutineContext().ensureActive()
                contentList.add(it)
            }
        }
        var contentStr = contentList.joinToString("\n")
        if (!book.isVideo && !book.isAudio && !book.isRss) contentStr =
            HtmlFormatter.formatKeepImg(contentStr, redirectUrl)
        //全文替换
        val replaceRegex = contentRule.replaceRegex
        if (!replaceRegex.isNullOrEmpty()) {
            contentStr = contentStr.split(AppPattern.LFRegex).joinToString("\n") { it.trim() }
            contentStr = analyzeRule.getString(replaceRegex, contentStr)
            contentStr = contentStr.split(AppPattern.LFRegex).joinToString("\n") { "　　$it" }
        }
        SourceDebugLoggers.impl?.log(bookSource.bookSourceUrl, "┌获取章节名称")
        SourceDebugLoggers.impl?.log(bookSource.bookSourceUrl, "└${bookChapter.title}")
        SourceDebugLoggers.impl?.log(bookSource.bookSourceUrl, "┌获取正文内容")
        SourceDebugLoggers.impl?.log(bookSource.bookSourceUrl, "└\n$contentStr")
        if (!bookChapter.isVolume && contentStr.isBlank()) {
            throw ContentEmptyException("内容为空")
        }
        return contentStr
    }

    @Throws(Exception::class)
    private suspend fun analyzeContent(
        book: Book,
        baseUrl: String,
        body: String,
        contentRule: ContentRule,
        chapter: BookChapter,
        bookSource: BookSource,
        nextChapterUrl: String?,
        getNextPageUrl: Boolean = true,
        printLog: Boolean = true,
        redirectUrl: String = baseUrl
    ): Pair<String, List<String>> {
        val analyzeRule = AnalyzeRuleFactories.create(book, bookSource)
        analyzeRule.setContent(body, baseUrl)
        analyzeRule.setRedirectUrl(redirectUrl)
        analyzeRule.coroutineContext = currentCoroutineContext()
        analyzeRule.nextChapterUrl = nextChapterUrl
        val nextUrlList = arrayListOf<String>()
        analyzeRule.chapter = chapter
        //获取正文
        val content = analyzeRule.getString(contentRule.content, unescape = false)
        //获取下一页链接
        if (getNextPageUrl) {
            val nextUrlRule = contentRule.nextContentUrl
            if (!nextUrlRule.isNullOrEmpty()) {
                SourceDebugLoggers.impl?.log(bookSource.bookSourceUrl, "┌获取正文下一页链接", printLog)
                analyzeRule.getStringList(nextUrlRule, isUrl = true)?.let {
                    nextUrlList.addAll(it)
                }
                SourceDebugLoggers.impl?.log(bookSource.bookSourceUrl, "└" + nextUrlList.joinToString("，"), printLog)
            }
        }
        return Pair(content, nextUrlList)
    }
}

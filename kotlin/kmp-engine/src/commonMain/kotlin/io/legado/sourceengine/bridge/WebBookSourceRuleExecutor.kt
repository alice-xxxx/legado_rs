package io.legado.sourceengine.bridge

import io.legado.app.data.entities.Book
import io.legado.app.data.entities.BookChapter
import io.legado.app.data.entities.BookSource
import io.legado.app.data.entities.OldRssSource
import io.legado.app.data.entities.SearchBook
import io.legado.app.data.entities.rule.ExploreKind
import io.legado.app.data.entities.toBookSource
import io.legado.app.help.source.exploreKinds
import io.legado.app.model.webBook.WebBook
import io.legado.app.utils.KS_JSON
import kotlinx.coroutines.withContext
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

/**
 * 把 Rust 请求映射到共享的 WebBook 编排入口。
 *
 * 解析顺序和字段语义继续由 WebBook / AnalyzeUrlCore 决定；这里仅负责稳定的跨语言参数与结果 JSON。
 * 宿主跟随本次协程传递，避免并发请求之间覆盖全局 HTTP 回调目标。
 */
object WebBookSourceRuleExecutor : SourceRuleExecutor {
    override suspend fun execute(call: SourceEngineCall, host: SourceEngineHost): String =
        SourceEngineHostRegistry.withHost(host) {
            withContext(SourceEngineHostContext(host)) {
                when (call.operation) {
                    "search" -> {
                        val source = decodeBookSource(call)
                        val page = WebBook.getBookListAwait(
                            bookSource = source,
                            key = call.keyword.orEmpty(),
                            page = call.page ?: 1,
                        )
                        buildJsonObject {
                            put(
                                "books",
                                JsonArray(page.books.map {
                                    KS_JSON.encodeToJsonElement(SearchBook.serializer(), it)
                                }),
                            )
                            put("hasNextPage", page.hasNextPage)
                        }.toString()
                    }

                    "exploreKinds" -> encodeExploreKinds(decodeBookSource(call))

                    "explore" -> encodeBookList(
                        WebBook.getBookListAwait(
                            bookSource = decodeBookSource(call),
                            key = call.keyword ?: error("Missing category URL for explore"),
                            page = call.page ?: 1,
                            isSearch = false,
                        ),
                    )

                    "rssExploreKinds" -> encodeExploreKinds(decodeRssSource(call))

                    "rssExplore" -> encodeBookList(
                        WebBook.getBookListAwait(
                            bookSource = decodeRssSource(call),
                            key = call.keyword ?: error("Missing category URL for RSS explore"),
                            page = call.page ?: 1,
                            isSearch = false,
                        ),
                    )

                    "rssBookInfo" -> {
                        val book = decodeBook(call)
                        KS_JSON.encodeToString(
                            Book.serializer(),
                            WebBook.getBookInfoAwait(decodeRssSource(call), book),
                        )
                    }

                    "rssChapters" -> {
                        val book = decodeBook(call)
                        val chapters = WebBook.getChapterListAwait(decodeRssSource(call), book).getOrThrow()
                        KS_JSON.encodeToString(ListSerializer(BookChapter.serializer()), chapters)
                    }

                    "rssContent" -> {
                        val book = decodeBook(call)
                        val chapter = decodeChapter(call)
                        JsonPrimitive(
                            WebBook.getContentAwait(
                                bookSource = decodeRssSource(call),
                                book = book,
                                bookChapter = chapter,
                                nextChapterUrl = call.nextChapterUrl,
                                needSave = false,
                            ),
                        ).toString()
                    }

                    "bookInfo" -> {
                        val book = decodeBook(call)
                        val source = decodeBookSource(call)
                        KS_JSON.encodeToString(Book.serializer(), WebBook.getBookInfoAwait(source, book))
                    }

                    "chapters" -> {
                        val book = decodeBook(call)
                        val source = decodeBookSource(call)
                        val chapters = WebBook.getChapterListAwait(source, book).getOrThrow()
                        KS_JSON.encodeToString(ListSerializer(BookChapter.serializer()), chapters)
                    }

                    "content" -> {
                        val book = decodeBook(call)
                        val chapter = decodeChapter(call)
                        val source = decodeBookSource(call)
                        JsonPrimitive(
                            WebBook.getContentAwait(
                                bookSource = source,
                                book = book,
                                bookChapter = chapter,
                                nextChapterUrl = call.nextChapterUrl,
                                // 此接口只返回章节正文；是否写入书架由上层阅读器决定。
                                needSave = false,
                            ),
                        ).toString()
                    }

                    else -> error("Unsupported source-engine operation: ${call.operation}")
                }
            }
        }

    private fun decodeBookSource(call: SourceEngineCall): BookSource =
        KS_JSON.decodeFromString(BookSource.serializer(), call.sourceJson)

    private fun decodeRssSource(call: SourceEngineCall): BookSource =
        KS_JSON.decodeFromString(OldRssSource.serializer(), call.sourceJson).toBookSource()

    private fun decodeBook(call: SourceEngineCall): Book =
        KS_JSON.decodeFromString(
            Book.serializer(),
            call.bookJson ?: error("Missing book JSON for ${call.operation}"),
        )

    private fun decodeChapter(call: SourceEngineCall): BookChapter =
        KS_JSON.decodeFromString(
            BookChapter.serializer(),
            call.chapterJson ?: error("Missing chapter JSON for ${call.operation}"),
        )

    private suspend fun encodeExploreKinds(source: BookSource): String =
        KS_JSON.encodeToString(ListSerializer(ExploreKind.serializer()), source.exploreKinds())

    private fun encodeBookList(page: io.legado.app.data.entities.BookListPage): String =
        buildJsonObject {
            put(
                "books",
                JsonArray(page.books.map {
                    KS_JSON.encodeToJsonElement(SearchBook.serializer(), it)
                }),
            )
            put("hasNextPage", page.hasNextPage)
        }.toString()
}

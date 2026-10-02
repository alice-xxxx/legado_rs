package io.legado.sourceengine.bridge

import io.legado.app.data.entities.Book
import io.legado.app.data.entities.BookChapter
import io.legado.app.data.entities.BookSource
import io.legado.app.data.entities.SearchBook
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
                val source = KS_JSON.decodeFromString(BookSource.serializer(), call.sourceJson)
                when (call.operation) {
                    "search" -> {
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

                    "bookInfo" -> {
                        val book = KS_JSON.decodeFromString(
                            Book.serializer(),
                            call.bookJson ?: error("Missing book JSON for bookInfo"),
                        )
                        KS_JSON.encodeToString(Book.serializer(), WebBook.getBookInfoAwait(source, book))
                    }

                    "chapters" -> {
                        val book = KS_JSON.decodeFromString(
                            Book.serializer(),
                            call.bookJson ?: error("Missing book JSON for chapters"),
                        )
                        val chapters = WebBook.getChapterListAwait(source, book).getOrThrow()
                        KS_JSON.encodeToString(ListSerializer(BookChapter.serializer()), chapters)
                    }

                    "content" -> {
                        val book = KS_JSON.decodeFromString(
                            Book.serializer(),
                            call.bookJson ?: error("Missing book JSON for content"),
                        )
                        val chapter = KS_JSON.decodeFromString(
                            BookChapter.serializer(),
                            call.chapterJson ?: error("Missing chapter JSON for content"),
                        )
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
}

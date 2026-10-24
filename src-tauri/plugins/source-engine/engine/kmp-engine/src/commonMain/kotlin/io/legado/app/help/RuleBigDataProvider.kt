package io.legado.app.help

import kotlin.concurrent.Volatile

interface RuleBigDataProvider {
    fun putBookVariable(bookUrl: String, key: String, value: String?)
    fun getBookVariable(bookUrl: String, key: String?): String?
    fun hasBookVariable(bookUrl: String, key: String): Boolean
    fun putChapterVariable(bookUrl: String, chapterUrl: String, key: String, value: String?)
    fun getChapterVariable(bookUrl: String, chapterUrl: String, key: String): String?

}

object RuleBigDataProviders {
    @Volatile
    var impl: RuleBigDataProvider? = null
}

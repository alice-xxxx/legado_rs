package io.legado.app.ui.rss

interface RssJsApi {

    /** 对照 `SearchActivity.start(activity, key)` */
    fun searchBook(key: String)

    /** 对照 `AddToBookshelfHelper.add(activity, bookUrl)` */
    fun addBook(bookUrl: String)
}

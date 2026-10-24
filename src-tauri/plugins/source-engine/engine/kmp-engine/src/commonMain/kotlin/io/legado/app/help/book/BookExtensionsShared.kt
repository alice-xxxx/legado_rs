@file:Suppress("unused")

package io.legado.app.help.book

import io.legado.app.constant.AppPattern
import io.legado.app.constant.BookSourceType
import io.legado.app.constant.BookType
import io.legado.app.data.entities.BaseBook
import io.legado.app.data.entities.Book
import io.legado.app.utils.MD5Utils
import kotlin.math.min

/*
 * Book 扩展函数下沉区 (shared jvmAndAndroidMain)。
 *
 * 仅放 shared 侧实体类 (Book/BaseBook 等) 自身依赖、且无 app 平台绑定的扩展。
 * 安卓侧绑定 (Uri/FileDoc/AppConfig 等) 留 app 端 BookExtensions.kt 同包合并。
 *
 * 跨模块同包名同签名扩展自动合并, 消费方 import 零改动。
 * 注意: 同包名同签名扩展函数不允许在两个模块同时定义, 需从 app 端删除已下沉的扩展。
 */

fun Book.getFolderNameNoCache(): String {
    return name.replace(AppPattern.fileNameRegex, "").let {
        it.substring(0, min(9, it.length)) + MD5Utils.md5Encode16(bookUrl)
    }
}

fun BaseBook.isType(bookType: Int): Boolean = type and bookType > 0

val BaseBook.isVideo: Boolean
    get() = isType(BookType.video)

val BaseBook.isAudio: Boolean
    get() = isType(BookType.audio)

val BaseBook.isImage: Boolean
    get() = isType(BookType.image)

val BaseBook.isRss: Boolean
    get() = isType(BookType.rss)

val BaseBook.isWebFile: Boolean
    get() = isType(BookType.webFile)

val BaseBook.isUpError: Boolean
    get() = isType(BookType.updateError)

val BaseBook.isArchive: Boolean
    get() = isType(BookType.archive)

val BaseBook.isNotShelf: Boolean
    get() = isType(BookType.notShelf)

val BaseBook.isLocal: Boolean
    get() {
        if (type == 0) {
            return origin == BookType.localTag || origin.startsWith(BookType.webDavTag)
        }
        return isType(BookType.local)
    }

val BaseBook.isLocalTxt: Boolean
    get() = isLocal && originName.endsWith(".txt", true)

val BaseBook.isEpub: Boolean
    get() = isLocal && originName.endsWith(".epub", true)

val BaseBook.isPdf: Boolean
    get() = isLocal && originName.endsWith(".pdf", true)

val BaseBook.isOnLineTxt: Boolean
    get() = !isLocal && isType(BookType.text)

val BaseBook.archiveName: String
    get() {
        if (!isArchive) throw io.legado.app.exception.NoStackTraceException("Book is not deCompressed from archive")
        // local_book::archive.rar
        // webDav::https://...../archive.rar
        return origin.substringAfter("::").substringAfterLast("/")
    }

fun BaseBook.getRemoteUrl(): String? {
    if (origin.startsWith(BookType.webDavTag)) {
        return origin.substring(BookType.webDavTag.length)
    }
    return null
}

fun BaseBook.setType(vararg types: Int) {
    type = 0
    addType(*types)
}

fun BaseBook.addType(vararg types: Int) {
    types.forEach {
        type = type or it
    }
}

fun BaseBook.removeType(vararg types: Int) {
    types.forEach {
        type = type and it.inv()
    }
}

fun BaseBook.removeAllBookType() {
    removeType(BookType.allBookType)
}

fun BaseBook.clearType() {
    type = 0
}

fun BaseBook.upType() {
    if (type < 8) {
        type = when (type) {
            BookSourceType.video -> BookType.video
            BookSourceType.image -> BookType.image
            BookSourceType.audio -> BookType.audio
            BookSourceType.file -> BookType.webFile
            else -> BookType.text
        }
        if (origin == BookType.localTag || origin.startsWith(BookType.webDavTag)) {
            type = type or BookType.local
        }
    }
}

fun BaseBook.releaseHtmlData() {
    infoHtml = null
    tocHtml = null
}

fun BaseBook.primaryStr(): String {
    return origin + bookUrl
}

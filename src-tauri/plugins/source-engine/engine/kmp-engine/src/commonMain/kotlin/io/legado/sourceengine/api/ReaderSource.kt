package io.legado.sourceengine.api

import io.legado.app.data.entities.Book
import io.legado.app.help.http.header
import io.legado.app.help.source.exploreKinds
import io.legado.app.help.tts.HttpTtsRequest
import io.legado.app.model.analyzeRule.AnalyzeUrlFactories
import io.legado.app.model.webBook.WebBook
import io.legado.sourceengine.host.*
import kotlinx.coroutines.currentCoroutineContext
import kotlin.io.encoding.Base64

/** 直接使用规则引擎；不读取书架，不保存目录和正文。 */
object ReaderSource {
    suspend fun search(r:SearchRequest):SearchPage {val page=WebBook.getBookListAwait(r.source,r.keyword,r.page);return SearchPage(page.books,page.hasNextPage)}
    suspend fun exploreKinds(r:SourceRequest)=ExploreKinds(r.source.exploreKinds())
    suspend fun explore(r:ExploreRequest):SearchPage {val page=WebBook.getBookListAwait(r.source,r.url,r.page,isSearch=false);return SearchPage(page.books,page.hasNextPage)}
    suspend fun detail(r:BookRequest):Book=WebBook.getBookInfoAwait(r.source,r.book)
    suspend fun chapters(r:BookRequest)=ChapterList(WebBook.getChapterListAwait(r.source,r.book).getOrThrow())
    suspend fun content(r:ContentRequest)=ChapterContent(WebBook.getContentAwait(r.source,r.book,r.chapter,r.nextChapterUrl))
    suspend fun resolveMedia(r:MediaRequest):MediaLocation {
        val(url,headers)=AnalyzeUrlFactories.create(rawUrl=r.url,source=r.source,ruleData=r.book,chapter=r.chapter,headerMapF=r.headers,coroutineContext=currentCoroutineContext()).resolveMedia()
        return MediaLocation(url,headers)
    }
    suspend fun httpTtsAudio(r:TtsRequest):AudioData {
        require(r.text.isNotBlank()&&r.text.length<=20000&&r.text.none{it=='\u0000'||(it.isISOControl()&&it !in "\n\r\t")}){"朗读文本无效"}
        require(r.speechRate in 0..45){"朗读速率无效"}
        val response=HttpTtsRequest.audioResponse(r.config,r.text,r.speechRate,currentCoroutineContext())
        return try {check(response.header("Content-Length")?.toLongOrNull()?.let{it<=16*1024*1024}!=false){"音频超过限制"}
            val bytes=response.body.bytes();check(bytes.isNotEmpty()&&bytes.size<=16*1024*1024){"音频大小无效"}
            AudioData(Base64.encode(bytes),response.header("Content-Type")?.substringBefore(';').orEmpty())
        }finally{response.close()}
    }
}

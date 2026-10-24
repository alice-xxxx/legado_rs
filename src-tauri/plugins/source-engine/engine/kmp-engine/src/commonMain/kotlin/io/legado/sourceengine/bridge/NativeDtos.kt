package io.legado.sourceengine.bridge

import io.legado.app.data.entities.Book
import io.legado.app.utils.KS_JSON
import kotlinx.serialization.encodeToString
import kotlinx.serialization.decodeFromString

/** Serialization is confined to the native ABI; engine methods take typed requests. */
object NativeDtos {
    @kotlin.jvm.JvmStatic @Throws(Exception::class)
    fun decodeSearchRequest(json: String): SearchRequest = KS_JSON.decodeFromString(json)
    @kotlin.jvm.JvmStatic @Throws(Exception::class)
    fun decodeSourceRequest(json: String): SourceRequest = KS_JSON.decodeFromString(json)
    @kotlin.jvm.JvmStatic @Throws(Exception::class)
    fun decodeExploreRequest(json: String): ExploreRequest = KS_JSON.decodeFromString(json)
    @kotlin.jvm.JvmStatic @Throws(Exception::class)
    fun decodeBookRequest(json: String): BookRequest = KS_JSON.decodeFromString(json)
    @kotlin.jvm.JvmStatic @Throws(Exception::class)
    fun decodeContentRequest(json: String): ContentRequest = KS_JSON.decodeFromString(json)
    @kotlin.jvm.JvmStatic @Throws(Exception::class)
    fun decodeMediaRequest(json: String): MediaRequest = KS_JSON.decodeFromString(json)
    @kotlin.jvm.JvmStatic @Throws(Exception::class)
    fun decodeTtsRequest(json: String): TtsRequest = KS_JSON.decodeFromString(json)
    @kotlin.jvm.JvmStatic @Throws(Exception::class)
    fun decodeLoginRequest(json: String): LoginRequest = KS_JSON.decodeFromString(json)
    @kotlin.jvm.JvmStatic @Throws(Exception::class)
    fun encodeSearchPage(value: SearchPage): String = KS_JSON.encodeToString(value)
    @kotlin.jvm.JvmStatic @Throws(Exception::class)
    fun encodeExploreKinds(value: ExploreKinds): String = KS_JSON.encodeToString(value)
    @kotlin.jvm.JvmStatic @Throws(Exception::class)
    fun encodeBook(value: Book): String = KS_JSON.encodeToString(value)
    @kotlin.jvm.JvmStatic @Throws(Exception::class)
    fun encodeChapterList(value: ChapterList): String = KS_JSON.encodeToString(value)
    @kotlin.jvm.JvmStatic @Throws(Exception::class)
    fun encodeChapterContent(value: ChapterContent): String = KS_JSON.encodeToString(value)
    @kotlin.jvm.JvmStatic @Throws(Exception::class)
    fun encodeMediaLocation(value: MediaLocation): String = KS_JSON.encodeToString(value)
    @kotlin.jvm.JvmStatic @Throws(Exception::class)
    fun encodeAudioData(value: AudioData): String = KS_JSON.encodeToString(value)
    @kotlin.jvm.JvmStatic @Throws(Exception::class)
    fun encodeLoginState(value: LoginState): String = KS_JSON.encodeToString(value)
}

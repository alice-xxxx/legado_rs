package io.legado.sourceengine.api

import io.legado.app.data.entities.Book
import kotlinx.serialization.*
import kotlinx.serialization.json.Json

/** 编解码只用于原生边界，规则接口始终传递数据类型。 */
object NativeCodec {
    private val json=Json{ignoreUnknownKeys=true;encodeDefaults=true;explicitNulls=false;coerceInputValues=true}
    @kotlin.jvm.JvmStatic @Throws(Exception::class) fun decodeSearchRequest(value:String):SearchRequest=json.decodeFromString(value)
    @kotlin.jvm.JvmStatic @Throws(Exception::class) fun decodeSourceRequest(value:String):SourceRequest=json.decodeFromString(value)
    @kotlin.jvm.JvmStatic @Throws(Exception::class) fun decodeExploreRequest(value:String):ExploreRequest=json.decodeFromString(value)
    @kotlin.jvm.JvmStatic @Throws(Exception::class) fun decodeBookRequest(value:String):BookRequest=json.decodeFromString(value)
    @kotlin.jvm.JvmStatic @Throws(Exception::class) fun decodeContentRequest(value:String):ContentRequest=json.decodeFromString(value)
    @kotlin.jvm.JvmStatic @Throws(Exception::class) fun decodeMediaRequest(value:String):MediaRequest=json.decodeFromString(value)
    @kotlin.jvm.JvmStatic @Throws(Exception::class) fun decodeTtsRequest(value:String):TtsRequest=json.decodeFromString(value)
    @kotlin.jvm.JvmStatic @Throws(Exception::class) fun decodeLoginRequest(value:String):LoginRequest=json.decodeFromString(value)
    @kotlin.jvm.JvmStatic @Throws(Exception::class) fun encodeSearchPage(value:SearchPage):String=json.encodeToString(value)
    @kotlin.jvm.JvmStatic @Throws(Exception::class) fun encodeExploreKinds(value:ExploreKinds):String=json.encodeToString(value)
    @kotlin.jvm.JvmStatic @Throws(Exception::class) fun encodeBook(value:Book):String=json.encodeToString(value)
    @kotlin.jvm.JvmStatic @Throws(Exception::class) fun encodeChapterList(value:ChapterList):String=json.encodeToString(value)
    @kotlin.jvm.JvmStatic @Throws(Exception::class) fun encodeChapterContent(value:ChapterContent):String=json.encodeToString(value)
    @kotlin.jvm.JvmStatic @Throws(Exception::class) fun encodeMediaLocation(value:MediaLocation):String=json.encodeToString(value)
    @kotlin.jvm.JvmStatic @Throws(Exception::class) fun encodeAudioData(value:AudioData):String=json.encodeToString(value)
    @kotlin.jvm.JvmStatic @Throws(Exception::class) fun encodeLoginState(value:LoginState):String=json.encodeToString(value)
}

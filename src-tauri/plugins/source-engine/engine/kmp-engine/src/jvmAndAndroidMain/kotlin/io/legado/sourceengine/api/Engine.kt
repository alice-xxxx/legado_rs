package io.legado.sourceengine.api

import io.legado.app.data.entities.Book
import io.legado.sourceengine.host.*
import kotlinx.coroutines.runBlocking

/** Android 与桌面共用 JNI 入口，生命周期和规则调用保持一致。 */
object Engine {
    private fun <T> call(directory:String,id:String,block:suspend()->T):T {
        HostRuntime.install(::image)
        return runBlocking { SourceEngineHostRegistry.withHost(JniHost(id,directory)){block()} }
    }
    @kotlin.jvm.JvmStatic @Throws(Exception::class) fun search(request:SearchRequest,directory:String,id:String):SearchPage=call(directory,id){ReaderSource.search(request)}
    @kotlin.jvm.JvmStatic @Throws(Exception::class) fun exploreKinds(request:SourceRequest,directory:String,id:String):ExploreKinds=call(directory,id){ReaderSource.exploreKinds(request)}
    @kotlin.jvm.JvmStatic @Throws(Exception::class) fun explore(request:ExploreRequest,directory:String,id:String):SearchPage=call(directory,id){ReaderSource.explore(request)}
    @kotlin.jvm.JvmStatic @Throws(Exception::class) fun detail(request:BookRequest,directory:String,id:String):Book=call(directory,id){ReaderSource.detail(request)}
    @kotlin.jvm.JvmStatic @Throws(Exception::class) fun chapters(request:BookRequest,directory:String,id:String):ChapterList=call(directory,id){ReaderSource.chapters(request)}
    @kotlin.jvm.JvmStatic @Throws(Exception::class) fun content(request:ContentRequest,directory:String,id:String):ChapterContent=call(directory,id){ReaderSource.content(request)}
    @kotlin.jvm.JvmStatic @Throws(Exception::class) fun resolveMedia(request:MediaRequest,directory:String,id:String):MediaLocation=call(directory,id){ReaderSource.resolveMedia(request)}
    @kotlin.jvm.JvmStatic @Throws(Exception::class) fun httpTtsAudio(request:TtsRequest,directory:String,id:String):AudioData=call(directory,id){ReaderSource.httpTtsAudio(request)}
    @kotlin.jvm.JvmStatic @Throws(Exception::class) fun login(request:LoginRequest,directory:String,id:String):LoginState=call(directory,id){SourceLogin.login(request)}
}

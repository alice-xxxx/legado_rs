package io.legado.sourceengine.api

import io.legado.app.data.entities.Book
import io.legado.sourceengine.host.*


/** iOS 具名入口，平台初始化不进入规则模块。 */
object Engine {
    fun isCancelled(id:String)=sourceExecutionCancelled(id)
    private suspend fun <T> call(directory:String,id:String,block:suspend()->T):T=SourceEngineHostRegistry.withHost(IosHost(id,directory)){
        HostRuntime.install()
        block()
    }
    @Throws(Throwable::class) suspend fun search(request:SearchRequest,directory:String,id:String):SearchPage=call(directory,id){ReaderSource.search(request)}
    @Throws(Throwable::class) suspend fun exploreKinds(request:SourceRequest,directory:String,id:String):ExploreKinds=call(directory,id){ReaderSource.exploreKinds(request)}
    @Throws(Throwable::class) suspend fun explore(request:ExploreRequest,directory:String,id:String):SearchPage=call(directory,id){ReaderSource.explore(request)}
    @Throws(Throwable::class) suspend fun detail(request:BookRequest,directory:String,id:String):Book=call(directory,id){ReaderSource.detail(request)}
    @Throws(Throwable::class) suspend fun chapters(request:BookRequest,directory:String,id:String):ChapterList=call(directory,id){ReaderSource.chapters(request)}
    @Throws(Throwable::class) suspend fun content(request:ContentRequest,directory:String,id:String):ChapterContent=call(directory,id){ReaderSource.content(request)}
    @Throws(Throwable::class) suspend fun resolveMedia(request:MediaRequest,directory:String,id:String):MediaLocation=call(directory,id){ReaderSource.resolveMedia(request)}
    @Throws(Throwable::class) suspend fun httpTtsAudio(request:TtsRequest,directory:String,id:String):AudioData=call(directory,id){ReaderSource.httpTtsAudio(request)}
    @Throws(Throwable::class) suspend fun login(request:LoginRequest,directory:String,id:String):LoginState=call(directory,id){SourceLogin.login(request)}
}

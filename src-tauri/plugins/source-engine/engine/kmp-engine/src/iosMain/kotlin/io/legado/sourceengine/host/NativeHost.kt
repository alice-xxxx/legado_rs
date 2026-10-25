@file:OptIn(kotlinx.cinterop.ExperimentalForeignApi::class)
package io.legado.sourceengine.host

import kotlinx.cinterop.*
import io.foliver.sourceengine.ffi.*
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Runnable
import kotlin.coroutines.CoroutineContext

@kotlin.native.concurrent.ThreadLocal private var local:SourceEngineHost?=null
internal actual fun currentHost()=local
internal actual fun setHost(host:SourceEngineHost?){local=host}
// Native 没有 JVM 的 ThreadContextElement，恢复调度片段自身的线程状态。
internal actual fun hostContext(host:SourceEngineHost,dispatcher:CoroutineDispatcher):CoroutineContext=object:CoroutineDispatcher(){
    override fun dispatch(context:CoroutineContext,block:Runnable){
        dispatcher.dispatch(context,Runnable{SourceEngineHostRegistry.onThread(host){block.run()}})
    }
}
internal actual fun sourceExecutionCancelled(id:String)=foliver_source_cancelled(id)!=0
private fun invoke(value:String,call:(String?)->CPointer<ByteVar>?):String{
    val result=call(value)?:error("原生能力未返回结果")
    return try{result.toKString()}finally{foliver_source_free(result)}
}
internal fun image(value:String)=invoke(value,::foliver_source_image)
internal class IosHost(id:String,directory:String):NativeHost(id,directory){
    override fun http(request:String)=invoke(request,::foliver_source_http)
    override fun store(request:String)=invoke(request,::foliver_source_storage)
    override suspend fun browser(request:String)=kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.Default){invoke(request,::foliver_source_browser)}
}

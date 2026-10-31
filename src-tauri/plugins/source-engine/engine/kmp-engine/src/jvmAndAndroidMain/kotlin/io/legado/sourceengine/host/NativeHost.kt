@file:JvmName("NativeCalls")
package io.legado.sourceengine.host

internal external fun http(input:String):String
internal external fun storage(input:String):String
internal external fun image(input:String):String
internal external fun cancelled(id:String):String
internal external fun browser(input:String):String
internal actual fun sourceExecutionCancelled(id:String)=cancelled(id)=="true"
internal class JniHost(id:String,directory:String):NativeHost(id,directory){
    override fun http(request:String)=io.legado.sourceengine.host.http(request)
    override fun store(request:String)=storage(request)
    override suspend fun browser(request:String)=io.legado.sourceengine.host.browser(request)
}

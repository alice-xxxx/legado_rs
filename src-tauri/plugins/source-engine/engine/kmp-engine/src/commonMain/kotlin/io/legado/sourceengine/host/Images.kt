package io.legado.sourceengine.host

import io.legado.app.help.image.*
import kotlinx.serialization.json.*
import kotlin.io.encoding.Base64

/** 图片引用在源调用结束时释放，显式脚本作用域可提前释放。 */
internal class NativeImages(private val image:(String)->String):ImageOps {
    internal class Picture(var png:String?):ImageRef
    internal class Scope {
        private val pictures=mutableListOf<Picture>()
        fun keep(picture:Picture){
            pictures.add(picture)
            check(pictures.sumOf{it.png?.length?.toLong()?:0}<=128*1024*1024){"图片调用超过限制"}
        }
        fun close(){pictures.forEach{it.png=null};pictures.clear()}
    }
    private var scope:MutableList<Picture>?=null
    private fun bytes(ref:ImageRef)=(ref as Picture).png?:error("图片作用域已结束")
    private fun call(op:String,fields:JsonObjectBuilder.()->Unit)=payload(image(buildJsonObject{put("op",op);fields()}.toString()))
    private fun ref(value:String):ImageRef {
        require(value.length<=90*1024*1024){"图片数据超限"}
        val ref=Picture(value)
        (SourceEngineHostRegistry.current() as NativeHost).imageScope.keep(ref)
        scope?.add(ref)
        return ref
    }
    private fun result(value:JsonObject)=ref(value.getValue("pngBase64").jsonPrimitive.content)
    override fun decode(bytes:ByteArray)=result(call("decode"){put("bytesBase64",Base64.encode(bytes))})
    override fun decode(base64:String)=decode(Base64.decode(base64.substringAfter("base64,",base64)))
    override fun encode(img:ImageRef,format:String,quality:Int)=Base64.decode(call("encode"){put("imageBase64",bytes(img));put("format",format);put("quality",quality.coerceIn(1,100))}.getValue("bytesBase64").jsonPrimitive.content)
    override fun crop(img:ImageRef,x:Int,y:Int,w:Int,h:Int)=result(call("crop"){put("imageBase64",bytes(img));put("x",x);put("y",y);put("w",w);put("h",h)})
    override fun rotate(img:ImageRef,deg:Int)=result(call("rotate"){put("imageBase64",bytes(img));put("deg",deg)})
    override fun flip(img:ImageRef,direction:String)=result(call("flip"){put("imageBase64",bytes(img));put("direction",direction)})
    override fun size(img:ImageRef)=call("size"){put("imageBase64",bytes(img))}.mapValues{it.value.jsonPrimitive.int}
    override fun split(img:ImageRef,rows:Int,cols:Int)=call("split"){put("imageBase64",bytes(img));put("rows",rows);put("cols",cols)}.getValue("imagesBase64").jsonArray.map{ref(it.jsonPrimitive.content)}
    override fun stitch(imgs:List<ImageRef>,direction:String)=result(call("stitch"){put("imagesBase64",JsonArray(imgs.map{JsonPrimitive(bytes(it))}));put("direction",direction)})
    override fun <T> withScope(block:()->T):T {
        val old=scope;val refs=mutableListOf<Picture>();scope=refs
        return try{block()}finally{refs.forEach{it.png=null};scope=old}
    }
}

package io.legado.sourceengine.compat

/** 桌面 JVM 保留源脚本常用的 Android Base64 静态接口。 */
object AndroidBase64 {
    const val DEFAULT=0;const val NO_PADDING=1;const val NO_WRAP=2;const val CRLF=4;const val URL_SAFE=8;const val NO_CLOSE=16
    @JvmStatic fun encodeToString(input:ByteArray,flags:Int):String {
        var encoder=if(flags and URL_SAFE!=0)java.util.Base64.getUrlEncoder()else java.util.Base64.getEncoder()
        if(flags and NO_PADDING!=0)encoder=encoder.withoutPadding()
        val encoded=encoder.encodeToString(input)
        if(flags and NO_WRAP!=0||encoded.isEmpty())return encoded
        val newline=if(flags and CRLF!=0)"\r\n" else "\n"
        return encoded.chunked(76).joinToString(newline)+newline
    }
    @JvmStatic fun encodeToString(input:ByteArray,offset:Int,length:Int,flags:Int)=encodeToString(input.copyOfRange(offset,offset+length),flags)
    @JvmStatic fun encode(input:ByteArray,flags:Int)=encodeToString(input,flags).toByteArray(Charsets.US_ASCII)
    @JvmStatic fun encode(input:ByteArray,offset:Int,length:Int,flags:Int)=encode(input.copyOfRange(offset,offset+length),flags)
    @JvmStatic fun decode(input:String,flags:Int):ByteArray=if(flags and URL_SAFE!=0)
        java.util.Base64.getUrlDecoder().decode(input.filter{it in 'A'..'Z'||it in 'a'..'z'||it in '0'..'9'||it=='-'||it=='_'||it=='='})
        else java.util.Base64.getMimeDecoder().decode(input)
    @JvmStatic fun decode(input:ByteArray,flags:Int)=decode(input.toString(Charsets.US_ASCII),flags)
    @JvmStatic fun decode(input:ByteArray,offset:Int,length:Int,flags:Int)=decode(input.copyOfRange(offset,offset+length),flags)
}

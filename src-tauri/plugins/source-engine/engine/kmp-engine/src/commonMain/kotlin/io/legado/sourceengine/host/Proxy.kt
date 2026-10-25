package io.legado.sourceengine.host

/** 源的旧式代理凭证转换为标准 URL；协议和实际代理行为由宿主处理。 */
internal fun hostProxyUrl(value:String?):String?{
    if(value.isNullOrBlank())return null
    val legacy=Regex("^(https?|socks4|socks5)://(.+):(\\d{1,5})@([^@]*)@([^@]*)$").matchEntire(value)?:return value
    val(scheme,host,port,user,password)=legacy.destructured
    return "$scheme://${proxyCredential(user)}:${proxyCredential(password)}@$host:$port"
}
private fun proxyCredential(value:String)=buildString{
    for(byte in value.encodeToByteArray()){
        val n=byte.toInt()and 255
        if(n in 65..90||n in 97..122||n in 48..57||n==45||n==46||n==95||n==126)append(n.toChar())
        else append('%').append(n.toString(16).uppercase().padStart(2,'0'))
    }
}

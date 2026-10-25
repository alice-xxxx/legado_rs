package io.legado.app.help.file

import kotlin.concurrent.Volatile

/** 引擎文件目录由宿主提供，不推断应用安装方式或外部存储。 */
interface AppFilesDir {
    val filesDir:String
    val cacheDir:String
}
object AppFilesDirs {
    @Volatile private var impl:AppFilesDir?=null
    fun register(provider:AppFilesDir){impl=provider}
    fun get():AppFilesDir=impl?:error("宿主未提供引擎文件目录")
}

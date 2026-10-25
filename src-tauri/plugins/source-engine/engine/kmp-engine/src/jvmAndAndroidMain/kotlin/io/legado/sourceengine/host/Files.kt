package io.legado.sourceengine.host

import io.legado.app.help.file.AppFilesDir
import io.legado.app.help.file.AppFilesDirs
import java.io.File

/** 文件能力只认当前调用注入的引擎目录，不读取原应用的全局数据目录。 */
internal fun installFiles(){AppFilesDirs.register(object:AppFilesDir{
    private fun directory(child:String):String{
        val root=requireNotNull(SourceEngineHostRegistry.current()).dataDirectory
        val file=File(root,child);check(file.isDirectory||file.mkdirs()){"无法创建引擎目录"};return file.canonicalPath
    }
    override val filesDir:String get()=directory("files")
    override val cacheDir:String get()=directory("cache")
})}

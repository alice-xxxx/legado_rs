@file:OptIn(kotlinx.cinterop.ExperimentalForeignApi::class)
package io.legado.sourceengine.host

import io.legado.app.help.file.AppFilesDir
import io.legado.app.help.file.AppFilesDirs
import platform.Foundation.NSFileManager

/** iOS 也使用调用方注入的引擎目录，不推断原应用的沙箱布局。 */
internal fun installFiles(){AppFilesDirs.register(object:AppFilesDir{
    private fun directory(child:String):String{
        val path=requireNotNull(SourceEngineHostRegistry.current()).dataDirectory.trimEnd('/')+"/"+child
        check(NSFileManager.defaultManager.createDirectoryAtPath(path,true,null,null)){"无法创建引擎目录"}
        return path
    }
    override val filesDir:String get()=directory("files")
    override val cacheDir:String get()=directory("cache")
})}

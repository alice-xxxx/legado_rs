package io.legado.sourceengine.host

import io.legado.app.help.file.AppFilesDir
import io.legado.app.help.file.AppFilesDirs
import io.legado.app.utils.canonicalizeExistingPath
import okio.FileSystem
import okio.Path.Companion.toPath

/** 每次访问都使用当前调用注入的目录，不缓存宿主或推断沙箱布局。 */
internal fun installFiles() {
    AppFilesDirs.register(object : AppFilesDir {
        private fun directory(child: String): String {
            val root = requireNotNull(SourceEngineHostRegistry.current()).dataDirectory.toPath()
            val path = root / child
            FileSystem.SYSTEM.createDirectories(path)
            val canonical = canonicalizeExistingPath(path)
            check(FileSystem.SYSTEM.metadata(canonical).isDirectory) { "无法创建引擎目录" }
            return canonical.toString()
        }
        override val filesDir: String get() = directory("files")
        override val cacheDir: String get() = directory("cache")
    })
}

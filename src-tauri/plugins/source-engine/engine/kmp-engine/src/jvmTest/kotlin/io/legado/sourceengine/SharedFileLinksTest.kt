package io.legado.sourceengine

import io.legado.app.help.FileUtilsCommon
import io.legado.app.utils.randomUUIDString
import io.legado.sourceengine.host.NativeHost
import io.legado.sourceengine.host.SourceEngineHostRegistry
import io.legado.sourceengine.host.installFiles
import okio.FileSystem
import okio.Path
import kotlin.test.*

class SharedFileLinksTest {
    @Test fun canonicalHostAliasesWorkAndLinksCannotEscapeThroughNewPaths() {
        val fs = FileSystem.SYSTEM
        val temporary = FileSystem.SYSTEM_TEMPORARY_DIRECTORY / "source-links-${randomUUIDString()}"
        fs.createDirectories(temporary / "engine/cache")
        fs.createDirectories(temporary / "outside/sub")
        fun link(path: Path, target: Path) {
            if (System.getProperty("os.name").startsWith("Windows")) {
                val process = ProcessBuilder("cmd", "/c", "mklink", "/J", path.toString(), target.toString())
                    .redirectErrorStream(true).start()
                val output = process.inputStream.bufferedReader().readText()
                check(process.waitFor() == 0) { output }
            } else fs.createSymlink(path, target)
        }
        try {
            link(temporary / "alias", temporary / "engine")
            link(temporary / "engine/cache/link", temporary / "outside/sub")
            val host = object : NativeHost("links", (temporary / "alias").toString()) {
                override fun http(request: String): String = error("Unexpected HTTP")
                override fun store(request: String): String = error("Unexpected storage")
                override suspend fun browser(request: String): String = error("Unexpected browser")
            }
            installFiles()
            SourceEngineHostRegistry.onThread(host) {
                val engine = fs.canonicalize(temporary / "engine")
                assertEquals((engine / "cache").toString(), FileUtilsCommon.getCachePath())
                assertEquals(listOf((engine / "cache/link").toString()), FileUtilsCommon.listSubDirs((engine / "cache").toString()))
                for (path in listOf("link/new/file", "link/../new", "missing/../link/new")) {
                    assertFailsWith<SecurityException>(path) { FileUtilsCommon.resolveCachePath(path) }
                }
                assertEquals((engine / "cache/new").toString(), FileUtilsCommon.resolveCachePath("new"))
            }
            assertTrue(fs.exists(temporary / "outside/sub"))
        } finally { fs.deleteRecursively(temporary) }
    }
}

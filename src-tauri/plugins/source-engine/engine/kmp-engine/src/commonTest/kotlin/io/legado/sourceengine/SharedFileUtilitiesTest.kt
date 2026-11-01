package io.legado.sourceengine

import io.legado.app.exception.SecurityException
import io.legado.app.help.FileUtilsCommon as Files
import io.legado.app.utils.InputStream
import io.legado.app.utils.randomUUIDString
import io.legado.sourceengine.host.NativeHost
import io.legado.sourceengine.host.SourceEngineHostRegistry
import io.legado.sourceengine.host.installFiles
import okio.FileSystem
import okio.IOException
import okio.Path
import okio.Path.Companion.toPath
import kotlin.test.*

class SharedFileUtilitiesTest {
    private val fs = FileSystem.SYSTEM

    private fun host(path: Path) = object : NativeHost("files", path.toString()) {
        override fun http(request: String): String = error("Unexpected HTTP")
        override fun store(request: String): String = error("Unexpected storage")
        override suspend fun browser(request: String): String = error("Unexpected browser")
    }

    private fun fixture(block: (Path) -> Unit) {
        val root = FileSystem.SYSTEM_TEMPORARY_DIRECTORY / "source-files-${randomUUIDString()}"
        fs.createDirectories(root)
        installFiles()
        try {
            SourceEngineHostRegistry.onThread(host(root)) { block(fs.canonicalize(root)) }
        } finally { fs.deleteRecursively(root) }
    }

    @Test fun pathsAllowNewFilesAndRejectTraversalAndSiblingPrefixes() = fixture { root ->
        assertEquals((root / "cache/new/part.txt").toString(), Files.resolveCachePath("new/part.txt"))
        assertEquals((root / "cache/part.txt").toString(), Files.resolveCachePath("/part.txt"))
        assertEquals((root / "files/part.txt").toString(), Files.resolveCachePath("../files/part.txt"))
        for (path in listOf("../../outside/file", "../../${root.name}-other/file", "new/../../../outside")) {
            assertEquals("非法路径", assertFailsWith<SecurityException> { Files.resolveCachePath(path) }.message)
        }
        assertTrue(Files.writeBytes(Files.resolveCachePath("valid"), byteArrayOf(42)))
        assertContentEquals(byteArrayOf(42), Files.readBytes(Files.resolveCachePath("valid")))
    }

    @Test fun directoriesFollowTheCurrentHostAndRestoreAfterFailure() = fixture { root ->
        val first = Files.getCachePath()
        SourceEngineHostRegistry.onThread(host(root / "second")) {
            assertEquals((root / "second/cache").toString(), Files.getCachePath())
            assertTrue(Files.writeBytes(Files.resolveCachePath("value"), byteArrayOf(2)))
        }
        assertFailsWith<IllegalStateException> {
            SourceEngineHostRegistry.onThread(host(root / "failed")) { Files.getCachePath(); error("fixture") }
        }
        assertEquals(first, Files.getCachePath())
        assertNull(Files.readBytes(Files.resolveCachePath("value")))
        assertTrue(Files.writeBytes((root / "blocked/cache").toString(), byteArrayOf(1)))
        SourceEngineHostRegistry.onThread(host(root / "blocked")) {
            assertFailsWith<IllegalStateException> { Files.getCachePath() }
        }
        assertEquals(first, Files.getCachePath())
    }

    @Test fun fileOperationsKeepReturnValuesAndDirectoryContents() = fixture { root ->
        val directory = Files.resolveCachePath("nested")
        val file = Files.getPath(directory, "", "one")
        assertTrue(Files.writeBytes(file, byteArrayOf(1, 2, 3)))
        assertTrue(Files.createFolderIfNotExist(directory))
        assertTrue(Files.createFolderIfNotExist(Files.getPath(directory, "child")))
        assertEquals(listOf(file), Files.listFiles(directory))
        assertEquals(listOf(Files.getPath(directory, "child")), Files.listSubDirs(directory))
        assertEquals(3L, Files.getDirSize(directory))
        assertEquals(3L, Files.getDirSize(file))
        assertTrue(Files.lastModified(file) > 0)
        assertTrue(Files.createFileReplace(file))
        assertContentEquals(byteArrayOf(), Files.readBytes(file))
        assertFalse(Files.writeBytes(Files.getPath(file, "child"), byteArrayOf(1)))
        assertTrue(Files.delete(directory, deleteRootDir = false))
        assertTrue(Files.exist(directory))
        assertEquals(emptyList(), Files.listFiles(directory))
        assertTrue(Files.delete(directory, deleteRootDir = false))
        assertTrue(Files.delete(directory))
        assertFalse(Files.delete(directory))
        assertNull(Files.readBytes(directory))
        assertEquals(0L, Files.getDirSize(directory))
        assertEquals(0L, Files.lastModified(directory))
        assertFalse(Files.writeBytes(root.toString(), byteArrayOf(1)))
        assertEquals("one", Files.getName("a\\b/one"))
        assertEquals("", Files.getName("a/"))
    }

    @Test fun downloadsStreamWithoutClosingTheInputAndRecoverFromReadFailure() = fixture {
        val bytes = ByteArray(200_000) { it.toByte() }
        val input = object : InputStream() {
            var offset = 0
            var closed = false
            var zeroRead = true
            override fun read(): Int = if (offset == bytes.size) -1 else bytes[offset++].toInt() and 255
            override fun read(target: ByteArray, start: Int, length: Int): Int {
                assertTrue(length <= 8192)
                if (zeroRead) { zeroRead = false; return 0 }
                if (offset == bytes.size) return -1
                val count = minOf(length, 511, bytes.size - offset)
                bytes.copyInto(target, start, offset, offset + count)
                offset += count
                return count
            }
            override fun close() { closed = true }
        }
        val path = Files.resolveCachePath("download/large")
        Files.copyToFile(path, input)
        assertFalse(input.closed)
        assertContentEquals(bytes, Files.readBytes(path))
        val failure = IOException("fixture")
        assertSame(failure, assertFailsWith<IOException> {
            Files.copyToFile(path, object : InputStream() { override fun read(): Int = throw failure })
        })
        assertTrue(Files.writeBytes(path, byteArrayOf(7)))
        assertContentEquals(byteArrayOf(7), Files.readBytes(path))
    }

    @Test fun relativeListingsStillReturnAbsolutePaths() {
        val relative = "source-files-${randomUUIDString()}".toPath()
        val root = fs.canonicalize(".".toPath()) / relative
        try {
            assertTrue(Files.writeBytes((relative / "file").toString(), byteArrayOf(1)))
            assertTrue(Files.createFolderIfNotExist((relative / "child").toString()))
            assertEquals(listOf((root / "file").toString()), Files.listFiles(relative.toString()))
            assertEquals(listOf((root / "child").toString()), Files.listSubDirs(relative.toString()))
        } finally { fs.deleteRecursively(root) }
    }
}

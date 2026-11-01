package io.legado.app.help

import io.legado.app.exception.SecurityException
import io.legado.app.help.file.AppFilesDirs
import io.legado.app.utils.InputStream
import io.legado.app.utils.asSource
import io.legado.app.utils.canonicalizeExistingPath
import okio.FileSystem
import okio.IOException
import okio.Path
import okio.Path.Companion.toPath

/** 源脚本文件 API。目录由当前宿主提供，文件操作统一使用 Okio。 */
object FileUtilsCommon {
    private val fs get() = FileSystem.SYSTEM

    fun getCachePath(): String = AppFilesDirs.get().cacheDir

    /** 保留缓存父目录内的相对访问；检查真实目录边界，包括尚未创建的子路径。 */
    fun resolveCachePath(relativePath: String): String {
        val cache = getCachePath().toPath()
        val root = canonicalizeExistingPath(cache.parent ?: cache)
        val path = canonicalPath("$cache${Path.DIRECTORY_SEPARATOR}$relativePath".toPath())
        if (path != root && generateSequence(path.parent) { it.parent }.none { it == root }) {
            throw SecurityException("非法路径")
        }
        return path.toString()
    }

    // Okio canonicalize 要求路径已存在；逐段解析以覆盖新文件及符号链接后的 ..。
    private fun canonicalPath(path: Path): Path {
        val absolute = absolutePath(path)
        return absolute.segments.fold(canonicalizeExistingPath(requireNotNull(absolute.root))) { parent, name ->
            val child = parent.resolve(name, normalize = true)
            if (fs.exists(child)) canonicalizeExistingPath(child) else child
        }
    }

    private fun absolutePath(path: Path): Path =
        if (path.isAbsolute) path else canonicalizeExistingPath(".".toPath()) / path

    private fun metadata(path: Path) = try {
        fs.metadataOrNull(canonicalizeExistingPath(path))
    } catch (_: IOException) { null }

    fun exist(path: String): Boolean = fs.exists(path.toPath())

    /** 读取失败返回 null，写入失败返回 false；下载流的 IO 异常直接交给调用方。 */
    fun readBytes(path: String): ByteArray? = try {
        fs.read(path.toPath()) { readByteArray() }
    } catch (_: Exception) { null }

    fun writeBytes(path: String, data: ByteArray): Boolean = try {
        val target = path.toPath()
        target.parent?.let { fs.createDirectories(it) }
        fs.write(target) { write(data) }
        true
    } catch (_: Exception) { false }

    /** 分块写入且只关闭输出，输入流的生命周期由调用方管理。 */
    fun copyToFile(path: String, input: InputStream) {
        val target = path.toPath()
        target.parent?.let { fs.createDirectories(it) }
        fs.write(target) { writeAll(input.asSource()) }
    }

    fun delete(path: String, deleteRootDir: Boolean = true): Boolean = try {
        val target = path.toPath()
        if (!fs.exists(target)) false else {
            if (deleteRootDir || !fs.metadata(target).isDirectory) fs.deleteRecursively(target)
            else fs.list(target).forEach { fs.deleteRecursively(it) }
            true
        }
    } catch (_: Exception) { false }

    fun listFiles(path: String): List<String> = fs.listOrNull(absolutePath(path.toPath())).orEmpty()
        .filter { metadata(it)?.isRegularFile == true }.map { it.toString() }

    fun listSubDirs(path: String): List<String> = fs.listOrNull(absolutePath(path.toPath())).orEmpty()
        .filter { metadata(it)?.isDirectory == true }.map { it.toString() }

    fun getPath(rootPath: String, vararg subDirFiles: String): String = buildString {
        append(rootPath)
        for (part in subDirFiles) if (part.isNotEmpty()) {
            if (!endsWith(Path.DIRECTORY_SEPARATOR)) append(Path.DIRECTORY_SEPARATOR)
            append(part)
        }
    }

    fun createFolderIfNotExist(path: String): Boolean = try {
        fs.createDirectories(path.toPath())
        true
    } catch (_: Exception) { false }

    fun createFileReplace(path: String): Boolean = try {
        val target = path.toPath()
        target.parent?.let { fs.createDirectories(it) }
        fs.delete(target)
        fs.write(target) { }
        true
    } catch (_: Exception) { false }

    fun getDirSize(path: String): Long {
        val target = path.toPath()
        val metadata = fs.metadataOrNull(target) ?: return 0L
        if (metadata.isRegularFile) return metadata.size ?: 0L
        return fs.listRecursively(target).sumOf {
            fs.metadataOrNull(it)?.takeIf { it.isRegularFile }?.size ?: 0L
        }
    }

    fun lastModified(path: String): Long = fs.metadataOrNull(path.toPath())?.lastModifiedAtMillis ?: 0L

    fun getName(path: String): String = path.substring(maxOf(path.lastIndexOf('/'), path.lastIndexOf('\\')) + 1)
}

package io.legado.app.help

import io.legado.app.help.FileUtilsCommon.createFileReplace
import io.legado.app.help.FileUtilsCommon.delete
import io.legado.app.help.FileUtilsCommon.getCachePath
import io.legado.app.help.FileUtilsCommon.getPath
import io.legado.app.help.FileUtilsCommon.readBytes
import io.legado.app.help.FileUtilsCommon.resolveCachePath
import io.legado.app.help.FileUtilsCommon.writeBytes
import io.legado.app.help.file.AppFilesDirs
import io.legado.app.utils.FileUtilsBase
import io.legado.app.utils.InputStream
import java.io.File

actual object FileUtilsCommon {

    actual fun getCachePath(): String {
        val dirs = AppFilesDirs.get()
        return dirs.cacheDir
    }

    actual fun resolveCachePath(relativePath: String): String {
        val cachePath = getCachePath()
        val aPath = if (relativePath.startsWith(File.separator)) {
            cachePath + relativePath
        } else {
            cachePath + File.separator + relativePath
        }
        val file = File(aPath)
        val safePath = File(cachePath).parent ?: cachePath
        if (!file.canonicalPath.startsWith(safePath)) {
            throw SecurityException("非法路径")
        }
        return file.absolutePath
    }

    actual fun exist(path: String): Boolean {
        return FileUtilsBase.exist(path)
    }

    actual fun readBytes(path: String): ByteArray? {
        return FileUtilsBase.readBytes(path)
    }

    actual fun writeBytes(path: String, data: ByteArray): Boolean {
        return FileUtilsBase.writeBytes(path, data)
    }

    actual fun copyToFile(path: String, input: InputStream) {
        val file = File(path)
        if (!file.exists()) {
            file.parentFile?.mkdirs()
            file.createNewFile()
        }
        file.outputStream().use { out -> input.copyTo(out) }
    }

    actual fun delete(path: String, deleteRootDir: Boolean): Boolean {
        return FileUtilsBase.delete(path, deleteRootDir)
    }

    actual fun listFiles(path: String): List<String> {
        val dir = File(path)
        if (!dir.isDirectory) return emptyList()
        val files = dir.listFiles { f -> f.isFile } ?: return emptyList()
        return files.map { it.absolutePath }
    }

    actual fun getPath(rootPath: String, vararg subDirFiles: String): String {
        return FileUtilsBase.getPath(rootPath, *subDirFiles)
    }

    actual fun createFolderIfNotExist(path: String): Boolean {
        FileUtilsBase.createFolderIfNotExist(path)
        return true
    }

    actual fun createFileReplace(path: String): Boolean {
        val file = File(path)
        return try {
            if (!file.exists()) {
                file.parent?.let { File(it).mkdirs() }
                file.createNewFile()
            } else {
                file.delete()
                file.createNewFile()
            }
            true
        } catch (_: Exception) {
            false
        }
    }

    actual fun listSubDirs(path: String): List<String> {
        val dir = File(path)
        if (!dir.isDirectory) return emptyList()
        return dir.listFiles { f -> f.isDirectory }?.map { it.absolutePath } ?: emptyList()
    }

    actual fun getDirSize(path: String): Long {
        val file = File(path)
        if (!file.exists()) return 0L
        if (file.isFile) return file.length()
        return file.walkTopDown().filter { it.isFile }.sumOf { it.length() }
    }

    actual fun lastModified(path: String): Long {
        val file = File(path)
        return if (file.exists()) file.lastModified() else 0L
    }

    actual fun getName(path: String): String {
        val idx = maxOf(path.lastIndexOf('/'), path.lastIndexOf('\\'))
        return if (idx >= 0) path.substring(idx + 1) else path
    }
}

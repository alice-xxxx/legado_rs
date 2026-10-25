package io.legado.app.help

import io.legado.app.exception.SecurityException
import io.legado.app.help.file.AppFilesDirs
import io.legado.app.utils.File
import io.legado.app.utils.InputStream

actual object FileUtilsCommon {

    actual fun getCachePath():String=AppFilesDirs.get().cacheDir

    actual fun resolveCachePath(relativePath: String): String {
        val cachePath = getCachePath()
        val aPath = if (relativePath.startsWith("/")) {
            cachePath + relativePath
        } else {
            "$cachePath/$relativePath"
        }
        val file = File(aPath)
        val safePath = File(cachePath).parent ?: cachePath
        if (!file.absolutePath.startsWith(safePath)) {
            throw SecurityException("非法路径")
        }
        return file.absolutePath
    }

    actual fun exist(path: String): Boolean {
        return File(path).exists()
    }

    actual fun readBytes(path: String): ByteArray? {
        return try {
            File(path).readBytes()
        } catch (_: Exception) {
            null
        }
    }

    actual fun writeBytes(path: String, data: ByteArray): Boolean {
        return try {
            val file = File(path)
            if (!file.exists()) {
                file.parent?.let { File(it).mkdirs() }
                file.createNewFile()
            }
            file.writeBytes(data)
            true
        } catch (_: Exception) {
            false
        }
    }

    actual fun copyToFile(path: String, input: InputStream) {
        val file = File(path)
        if (!file.exists()) {
            file.parent?.let { File(it).mkdirs() }
            file.createNewFile()
        }
        val buffer = ByteArray(64 * 1024)
        val chunks = ArrayList<ByteArray>()
        while (true) {
            val n = input.read(buffer)
            if (n <= 0) break
            chunks.add(buffer.copyOfRange(0, n))
        }
        val total = chunks.sumOf { it.size }
        val all = ByteArray(total)
        var offset = 0
        for (chunk in chunks) {
            chunk.copyInto(all, offset)
            offset += chunk.size
        }
        file.writeBytes(all)
    }

    actual fun delete(path: String, deleteRootDir: Boolean): Boolean {
        val file = File(path)
        if (!file.exists()) return false
        return try {
            if (file.isDirectory) {
                file.listFiles()?.forEach { child ->
                    delete(child.absolutePath, deleteRootDir = true)
                }
                if (!deleteRootDir) return true
            }
            file.delete()
        } catch (_: Exception) {
            false
        }
    }

    actual fun listFiles(path: String): List<String> {
        val dir = File(path)
        if (!dir.isDirectory) return emptyList()
        val files = dir.listFiles() ?: return emptyList()
        return files.filter { it.isFile }.map { it.absolutePath }
    }

    actual fun getPath(rootPath: String, vararg subDirFiles: String): String {
        val sb = StringBuilder(rootPath)
        subDirFiles.forEach {
            if (it.isNotEmpty()) {
                if (!sb.endsWith("/")) {
                    sb.append("/")
                }
                sb.append(it)
            }
        }
        return sb.toString()
    }

    actual fun createFolderIfNotExist(path: String): Boolean {
        return try {
            File(path).mkdirs()
        } catch (_: Exception) {
            false
        }
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
        return dir.listFiles()?.filter { it.isDirectory }?.map { it.absolutePath } ?: emptyList()
    }

    actual fun getDirSize(path: String): Long {
        val file = File(path)
        if (!file.exists()) return 0L
        if (file.isFile) return file.length()
        var size = 0L
        file.walkTopDown().forEach { if (it.isFile) size += it.length() }
        return size
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

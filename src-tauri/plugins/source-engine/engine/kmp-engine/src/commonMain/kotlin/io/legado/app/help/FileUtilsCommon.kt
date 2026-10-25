package io.legado.app.help

import io.legado.app.utils.InputStream

/** 源脚本的文件 API；目录与文件访问由平台实现及宿主目录端口提供。 */
expect object FileUtilsCommon {

    /** 当前源调用的私有缓存目录。 */
    fun getCachePath(): String

    /**
     * 解析缓存相对路径为绝对路径, 含安全检查 (防路径穿越)。
     *
     * - path 以分隔符开头: cachePath + path
     * - 否则: cachePath + separator + path
     * - 校验解析后的 canonicalPath 必须以 cachePath 的父目录为前缀, 否则抛 SecurityException
     *   (与原 app 端 JsExtensions.getFile 安全检查一致)
     */
    fun resolveCachePath(relativePath: String): String

    /** 判断文件/目录是否存在。 */
    fun exist(path: String): Boolean

    /** 读取文件全部字节, 失败返回 null (与原 FileUtilsBase.readBytes 行为一致)。 */
    fun readBytes(path: String): ByteArray?

    /** 写入字节到文件 (覆盖), 自动创建父目录, 成功返回 true。 */
    fun writeBytes(path: String, data: ByteArray): Boolean

    /**
     * 从流写入文件 (流式分块, 不整块缓冲), 自动创建父目录, 成功返回 true。
     *
     * 对齐原 app 端 JsExtensions.downloadFile 的 `getInputStream().copyTo` 语义
     * (大文件避免全量字节数组内存峰值)。
     *
     * IO 失败**直接抛出**, 不收成返回值 —— 原版流式失败即失败, 书源 JS 侧要能看到异常;
     * 收成 Boolean 会让调用方只能造一个丢掉原始类型与堆栈的替代异常。
     */
    fun copyToFile(path: String, input: InputStream)

    /**
     * 删除文件或目录。
     * - 文件: 直接删除
     * - 目录: 递归删除子项, deleteRootDir 控制是否删除目录本身
     *
     * 成功返回 true (与原 FileUtilsBase.delete 行为一致)。
     */
    fun delete(path: String, deleteRootDir: Boolean = true): Boolean

    /**
     * 列出目录下所有文件 (不含子目录) 的绝对路径。
     * 非目录或不存在返回空列表 (与原 folder.listFiles 行为对齐)。
     */
    fun listFiles(path: String): List<String>

    /** 拼接路径, 用平台分隔符 (与原 FileUtilsBase.getPath(root, vararg) 行为一致)。 */
    fun getPath(rootPath: String, vararg subDirFiles: String): String

    /** 创建文件夹 (含父目录), 已存在则不报错, 成功返回 true。 */
    fun createFolderIfNotExist(path: String): Boolean

    /**
     * 创建文件 (含父目录), 已存在则先删除再创建 (与原 File.createFileReplace 扩展行为一致)。
     * 成功返回 true。
     */
    fun createFileReplace(path: String): Boolean

    /**
     * 列出目录下所有一级子目录的绝对路径 (不含文件)。
     * 非目录或不存在返回空列表。
     */
    fun listSubDirs(path: String): List<String>

    /**
     * 递归统计目录下所有普通文件的大小总和 (字节); 文件返回自身大小, 不存在返回 0。
     */
    fun getDirSize(path: String): Long

    /** 文件/目录的最后修改时间戳 (毫秒, epoch), 不存在返回 0。 */
    fun lastModified(path: String): Long

    /**
     * 取路径最后一段名称 (跨平台: 兼容 '/' 与 '\\' 分隔符, 纯字符串操作)。
     * 尾部带分隔符的路径返回空串。
     */
    fun getName(path: String): String
}

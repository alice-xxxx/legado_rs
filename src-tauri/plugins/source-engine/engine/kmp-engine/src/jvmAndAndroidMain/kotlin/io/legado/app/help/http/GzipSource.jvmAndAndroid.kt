package io.legado.app.help.http

import okio.BufferedSource
import okio.Source
import okio.source
import java.util.zip.GZIPInputStream

// JDK 支持连接的 gzip 成员；Okio GzipSource 会拒绝成员后的剩余字节。
internal actual fun gzipSource(source: BufferedSource): Source = GZIPInputStream(source.inputStream()).source()

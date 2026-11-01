package io.legado.app.help.http

import okio.BufferedSource
import okio.GzipSource
import okio.Source

internal actual fun gzipSource(source: BufferedSource): Source = GzipSource(source)

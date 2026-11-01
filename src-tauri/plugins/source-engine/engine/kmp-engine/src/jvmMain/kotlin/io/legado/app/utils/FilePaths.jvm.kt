package io.legado.app.utils

import okio.Path
import okio.Path.Companion.toOkioPath

internal actual fun canonicalizeExistingPath(path: Path): Path = path.toNioPath().toRealPath().toOkioPath()

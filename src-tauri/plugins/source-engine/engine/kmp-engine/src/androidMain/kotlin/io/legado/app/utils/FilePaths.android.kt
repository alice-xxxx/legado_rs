package io.legado.app.utils

import okio.FileSystem
import okio.Path

internal actual fun canonicalizeExistingPath(path: Path): Path = FileSystem.SYSTEM.canonicalize(path)

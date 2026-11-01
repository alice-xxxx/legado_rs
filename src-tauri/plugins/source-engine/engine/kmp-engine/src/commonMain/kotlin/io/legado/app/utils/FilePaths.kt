package io.legado.app.utils

import okio.Path

/** 解析已存在路径的真实位置；Windows 目录连接需要 JVM 的 toRealPath。 */
internal expect fun canonicalizeExistingPath(path: Path): Path

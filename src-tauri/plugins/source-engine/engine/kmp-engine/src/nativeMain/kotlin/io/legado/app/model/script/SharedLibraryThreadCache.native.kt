package io.legado.app.model.script

@kotlin.native.concurrent.ThreadLocal
private var libraryCache: SharedLibraryThreadCache? = null
internal actual fun sharedLibraryThreadCache(): SharedLibraryThreadCache =
    libraryCache ?: SharedLibraryThreadCache().also { libraryCache = it }

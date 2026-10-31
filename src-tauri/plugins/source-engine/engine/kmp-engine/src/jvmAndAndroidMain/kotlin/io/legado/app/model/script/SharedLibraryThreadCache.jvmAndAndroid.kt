package io.legado.app.model.script

private val libraryCache = ThreadLocal.withInitial { SharedLibraryThreadCache() }
internal actual fun sharedLibraryThreadCache(): SharedLibraryThreadCache =
    libraryCache.get() ?: SharedLibraryThreadCache().also(libraryCache::set)

package io.legado.app.utils

internal actual fun createWordCountFormatter(): (Double) -> String = ::formatWordCountPlain

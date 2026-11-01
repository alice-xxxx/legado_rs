package io.legado.app.model.script

internal actual fun Char.isBindingIdentifierStart(): Boolean = isJavaIdentifierStart()
internal actual fun Char.isBindingIdentifierPart(): Boolean = isJavaIdentifierPart()

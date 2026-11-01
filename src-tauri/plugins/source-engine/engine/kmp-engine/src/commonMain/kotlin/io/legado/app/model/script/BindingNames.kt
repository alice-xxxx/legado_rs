package io.legado.app.model.script

internal fun isValidBindingName(name: String): Boolean =
    name.isNotEmpty() && name[0].isBindingIdentifierStart() && (1 until name.length).all { name[it].isBindingIdentifierPart() }

internal expect fun Char.isBindingIdentifierStart(): Boolean
internal expect fun Char.isBindingIdentifierPart(): Boolean

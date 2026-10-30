package io.legado.app.model.script

/** Rhino 的 Java 字符串方法也可用于原生 JS 字符串。 */
internal const val JAVA_STRING_COMPAT = """
Object.defineProperty(String.prototype, 'contains', {
    value: function(value) { return this.includes(String(value)); }, configurable: true, writable: true
});
Object.defineProperty(String.prototype, 'getBytes', {
    value: function(charset) { return java.strToBytes(String(this), charset == null ? 'UTF-8' : String(charset)); }, configurable: true, writable: true
});
"""

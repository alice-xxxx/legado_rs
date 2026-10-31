package io.legado.app.constant

import io.legado.app.utils.NumericDateFormat

class ThreadSafeDateFormat(pattern: String) {
    private val format = NumericDateFormat(pattern)
    fun format(millis: Long): String = format.format(millis)
}

private val timeFormatInternal by lazy { ThreadSafeDateFormat("HH:mm") }
private val dateFormatInternal by lazy { ThreadSafeDateFormat("yyyy/MM/dd HH:mm") }
private val fileNameFormatInternal by lazy { ThreadSafeDateFormat("yy-MM-dd-HH-mm-ss") }

val AppConst.timeFormat: ThreadSafeDateFormat get() = timeFormatInternal

val AppConst.dateFormat: ThreadSafeDateFormat get() = dateFormatInternal

val AppConst.fileNameFormat: ThreadSafeDateFormat get() = fileNameFormatInternal

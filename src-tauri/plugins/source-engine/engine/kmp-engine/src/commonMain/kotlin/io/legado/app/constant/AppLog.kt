package io.legado.app.constant

import io.legado.app.constant.AppLog.lock
import io.legado.app.constant.AppLog.put
import kotlinx.atomicfu.locks.SynchronizedObject
import kotlinx.atomicfu.locks.synchronized
import kotlin.concurrent.Volatile
import io.legado.app.utils.systemCurrentTimeMillis

/** 保留源脚本的日志接口和有界历史，展示与持久化交给宿主。 */
object AppLog {

    private val lock = SynchronizedObject()
    private val mLogs = ArrayList<Triple<Long, String, Throwable?>>()

    val logs get() = synchronized(lock) { mLogs.toList() }

    @Volatile
    private var host: AppLogHost? = null

    fun registerHost(appLogHost: AppLogHost) {
        host = appLogHost
    }

    fun put(
        message: String?,
        throwable: Throwable? = null,
        toast: Boolean = false,
        tag: String = "AppLog"
    ) {
        message ?: return
        val h = host
        synchronized(lock) {
            val now = h?.currentTimeMillis() ?: systemCurrentTimeMillis()
            if (toast) h?.toast(message)
            val logMessage = if (throwable == null) {
                message
            } else {
                "$message\n${throwable.stackTraceToString()}"
            }
            h?.write(tag, logMessage)
            if (mLogs.size > 100) {
                mLogs.removeLastOrNull()
            }
            mLogs.add(0, Triple(now, message, throwable))
            h?.debugPrint(tag, message, throwable)
        }
    }
    fun put(message: String?) = put(message, null, false, "AppLog")
    fun put(message: String?, throwable: Throwable?) = put(message, throwable, false, "AppLog")
    fun put(message: String?, throwable: Throwable?, toast: Boolean) =
        put(message, throwable, toast, "AppLog")

    fun putNotSave(
        message: String?,
        throwable: Throwable? = null,
        toast: Boolean = false,
        tag: String = "AppLog"
    ) {
        message ?: return
        val h = host
        synchronized(lock) {
            val now = h?.currentTimeMillis() ?: 0L
            if (toast) h?.toast(message)
            if (mLogs.size > 100) {
                mLogs.removeLastOrNull()
            }
            mLogs.add(0, Triple(now, message, throwable))
            h?.debugPrint(tag, message, throwable)
        }
    }
    fun putNotSave(message: String?) = putNotSave(message, null, false, "AppLog")
    fun putNotSave(message: String?, throwable: Throwable?) =
        putNotSave(message, throwable, false, "AppLog")

    fun putNotSave(message: String?, throwable: Throwable?, toast: Boolean) =
        putNotSave(message, throwable, toast, "AppLog")

    fun clear() {
        synchronized(lock) {
            mLogs.clear()
        }
    }

    fun putDebug(
        message: String?,
        throwable: Throwable? = null,
        tag: String = "AppLog"
    ) {
        if (host?.recordLog == true) {
            put(message, throwable, tag = tag)
        }
    }
    fun putDebug(message: String?) = putDebug(message, null, "AppLog")
    fun putDebug(message: String?, throwable: Throwable?) = putDebug(message, throwable, "AppLog")

    val isRecordLogEnabled: Boolean get() = host?.recordLog == true

    fun timeZoneOffsetMillis(): Long = host?.timeZoneOffsetMillis() ?: 0L

}

interface AppLogHost {

    fun currentTimeMillis(): Long

    fun timeZoneOffsetMillis(): Long
    val recordLog: Boolean
    fun write(tag: String, message: String)
    fun toast(message: String)
    fun debugPrint(tag: String, message: String, throwable: Throwable?)
}

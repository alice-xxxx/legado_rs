package io.legado.app.help

import io.legado.app.help.http.SSLHelper
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.SimpleTimeZone

internal actual object JsPlatformRuntime {
    actual fun formatTimeUtc(time: Long, format: String, shiftHours: Int): String {
        val utc = SimpleTimeZone(shiftHours, "UTC")
        return SimpleDateFormat(format, Locale.getDefault()).run {
            timeZone = utc
            format(Date(time))
        }
    }

    actual fun isMainThread(): Boolean {
        // jvmAndAndroidMain 共享 Android 与桌面 JVM:
        // - Android: 反射调用 Looper.getMainLooper().thread === Thread.currentThread()
        //   (行为对齐原 app 端 io.legado.app.utils.isMainThread: mainLooper.thread === currentThread)
        // - 桌面 JVM: 无 android.os.Looper, 反射失败兜底返回 false (无主线程概念)
        // 注: 直接 import android.os.Looper 会破坏 JVM 桌面端编译, 故走反射
        return try {
            val looperClass = Class.forName("android.os.Looper")
            val mainLooper = looperClass.getMethod("getMainLooper").invoke(null)
            val mainThread = mainLooper?.javaClass?.getMethod("getThread")?.invoke(mainLooper) as? Thread
            mainThread === Thread.currentThread()
        } catch (_: Throwable) {
            false
        }
    }

    actual fun unsafeSslContext(): Any? = SSLHelper.unsafeSslContext
}

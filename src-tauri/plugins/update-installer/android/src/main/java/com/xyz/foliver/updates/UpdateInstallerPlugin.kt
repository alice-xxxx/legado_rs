package com.xyz.foliver.updates

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.provider.Settings
import androidx.core.content.FileProvider
import androidx.core.content.pm.PackageInfoCompat
import app.tauri.plugin.JSObject
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin
import java.io.File

@TauriPlugin
class UpdateInstallerPlugin(private val activity: Activity) : Plugin(activity) {
    @Command
    @Suppress("DEPRECATION") // The int flag overloads also support Android 8-12.
    fun install(invoke: Invoke) {
        try {
            val apk = File(activity.cacheDir, "foliver-updates/update.apk")
            val manager = activity.packageManager
            val archive = manager.getPackageArchiveInfo(apk.path, 0)
                ?: error("安装包无法读取，请重新下载")
            val installed = manager.getPackageInfo(activity.packageName, 0)
            check(archive.packageName == activity.packageName) { "安装包不属于当前应用" }
            check(PackageInfoCompat.getLongVersionCode(archive) > PackageInfoCompat.getLongVersionCode(installed)) {
                "安装包版本不高于当前版本，请重新检查更新"
            }
            if (!manager.canRequestPackageInstalls()) {
                activity.startActivity(Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                    Uri.parse("package:${activity.packageName}")))
                invoke.resolve(JSObject().apply { put("permissionRequired", true) })
                return
            }
            val uri = FileProvider.getUriForFile(activity, "${activity.packageName}.updates", apk)
            activity.startActivity(Intent(Intent.ACTION_VIEW).apply {
                setDataAndType(uri, "application/vnd.android.package-archive")
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            })
            invoke.resolve(JSObject().apply { put("permissionRequired", false) })
        } catch (error: Exception) {
            invoke.reject(error.message ?: "无法启动系统安装器")
        }
    }
}

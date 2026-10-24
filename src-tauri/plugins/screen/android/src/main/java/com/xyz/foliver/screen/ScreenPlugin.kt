package com.xyz.foliver.screen

import android.app.Activity
import android.content.pm.ActivityInfo
import android.content.res.Configuration
import android.provider.Settings
import android.view.WindowManager
import android.webkit.WebView
import androidx.appcompat.app.AppCompatActivity
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import org.json.JSONArray

@InvokeArg class ScreenOptions {
    var fullscreen: Boolean? = null
    var systemUiHidden: Boolean? = null
    var systemUiStyle: String? = null
    var orientation: String? = null
    var keepAwake: Boolean? = null
    var brightness: Double? = null
}
@InvokeArg class ApplyArgs { var options = ScreenOptions() }

@TauriPlugin
class ScreenPlugin(private val activity: Activity) : Plugin(activity) {
    private var webview: WebView? = null
    private var originalBars: Int? = null
    private var originalBarsBehavior: Int? = null
    private var originalStyle: Pair<Boolean, Boolean>? = null
    private var originalOrientation: Int? = null
    private var originalAwake: Boolean? = null
    private var originalBrightness: Float? = null
    private val window get() = activity.window
    private val controller get() = WindowCompat.getInsetsController(window, window.decorView)
    private val barTypes = intArrayOf(WindowInsetsCompat.Type.statusBars(), WindowInsetsCompat.Type.navigationBars(), WindowInsetsCompat.Type.captionBar())

    override fun load(webView: WebView) { webview = webView }

    private fun visibleBars(): Int {
        val insets = ViewCompat.getRootWindowInsets(window.decorView)
        return barTypes.filter { insets?.isVisible(it) ?: (it != WindowInsetsCompat.Type.captionBar()) }.fold(0) { mask, type -> mask or type }
    }
    private fun setHidden(hidden: Boolean) {
        if (originalBars == null) originalBars = visibleBars()
        if (originalBarsBehavior == null) originalBarsBehavior = controller.systemBarsBehavior
        controller.systemBarsBehavior = androidx.core.view.WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        if (hidden) controller.hide(WindowInsetsCompat.Type.systemBars()) else controller.show(WindowInsetsCompat.Type.systemBars())
    }
    private fun setOrientation(value: String) {
        if (value == "restore") {
            originalOrientation?.let { activity.requestedOrientation = it }
            originalOrientation = null
            return
        }
        val target = when (value) {
            "portrait" -> ActivityInfo.SCREEN_ORIENTATION_SENSOR_PORTRAIT
            "landscape" -> ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE
            "portrait-primary" -> ActivityInfo.SCREEN_ORIENTATION_PORTRAIT
            "portrait-secondary" -> ActivityInfo.SCREEN_ORIENTATION_REVERSE_PORTRAIT
            "landscape-primary" -> ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE
            "landscape-secondary" -> ActivityInfo.SCREEN_ORIENTATION_REVERSE_LANDSCAPE
            "auto" -> ActivityInfo.SCREEN_ORIENTATION_FULL_USER
            else -> throw IllegalArgumentException("Unsupported orientation")
        }
        if (originalOrientation == null) originalOrientation = activity.requestedOrientation
        activity.requestedOrientation = target
    }
    private fun setBrightness(level: Double) {
        val attributes = window.attributes
        if (level == -1.0) {
            attributes.screenBrightness = originalBrightness ?: return
            originalBrightness = null
        } else {
            if (originalBrightness == null) originalBrightness = attributes.screenBrightness
            attributes.screenBrightness = level.toFloat()
        }
        window.attributes = attributes
    }
    private fun onMain(invoke: Invoke, action: () -> Unit) {
        activity.runOnUiThread { try { action() } catch (error: Exception) { invoke.reject(error.message ?: error.toString()) } }
    }

    @Command fun apply(invoke: Invoke) {
        val options = invoke.parseArgs(ApplyArgs::class.java).options
        onMain(invoke) {
            val applied = mutableListOf<String>()
            for ((key, value) in listOf("fullscreen" to options.fullscreen, "systemUiHidden" to options.systemUiHidden)) {
                value?.let { setHidden(it); applied.add(key) }
            }
            options.systemUiStyle?.let { style ->
                if (originalStyle == null) originalStyle = controller.isAppearanceLightStatusBars to controller.isAppearanceLightNavigationBars
                val darkIcons = style == "dark" || style == "auto" && activity.resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK != Configuration.UI_MODE_NIGHT_YES
                controller.isAppearanceLightStatusBars = darkIcons
                controller.isAppearanceLightNavigationBars = darkIcons
                applied.add("systemUiStyle")
            }
            options.orientation?.let { setOrientation(it); applied.add("orientation") }
            options.keepAwake?.let { enabled ->
                if (originalAwake == null) originalAwake = window.attributes.flags and WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON != 0
                if (enabled) window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON) else window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                applied.add("keepAwake")
            }
            options.brightness?.let { setBrightness(it); applied.add("brightness") }
            invoke.resolve(JSObject().apply { put("applied", JSONArray(applied)); put("unsupported", JSONArray()) })
        }
    }

    @Command fun getInfo(invoke: Invoke) = onMain(invoke) {
        val density = activity.resources.displayMetrics.density.toDouble()
        val insets = ViewCompat.getRootWindowInsets(window.decorView)?.getInsets(WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout())
        val view = webview ?: window.decorView
        val position = IntArray(2); val origin = IntArray(2)
        view.getLocationInWindow(position); window.decorView.getLocationInWindow(origin)
        val left = position[0] - origin[0] + view.paddingLeft
        val top = position[1] - origin[1] + view.paddingTop
        val width = (view.width - view.paddingLeft - view.paddingRight).coerceAtLeast(0)
        val height = (view.height - view.paddingTop - view.paddingBottom).coerceAtLeast(0)
        val safeTop = ((insets?.top ?: 0) - top).coerceAtLeast(0)
        val safeLeft = ((insets?.left ?: 0) - left).coerceAtLeast(0)
        val safeRight = ((insets?.right ?: 0) - (window.decorView.width - left - width)).coerceAtLeast(0)
        val safeBottom = ((insets?.bottom ?: 0) - (window.decorView.height - top - height)).coerceAtLeast(0)
        val level = window.attributes.screenBrightness
        invoke.resolve(JSObject().apply {
            put("width", width / density)
            put("height", height / density)
            put("scale", density)
            put("fullscreen", visibleBars() == 0); put("systemUiHidden", visibleBars() == 0)
            put("systemUiStyle", if (controller.isAppearanceLightStatusBars) "dark" else "light")
            put("orientation", if (activity.resources.configuration.orientation == Configuration.ORIENTATION_LANDSCAPE) "landscape" else "portrait")
            put("keepAwake", window.attributes.flags and WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON != 0)
            put("brightness", if (level >= 0) level.toDouble() else Settings.System.getInt(activity.contentResolver, Settings.System.SCREEN_BRIGHTNESS, 128) / 255.0)
            put("safeArea", JSObject().apply { put("top", safeTop / density); put("right", safeRight / density); put("bottom", safeBottom / density); put("left", safeLeft / density) })
            put("capabilities", JSObject().apply { for (name in listOf("fullscreen", "systemUiHidden", "systemUiStyle", "orientation", "keepAwake", "brightness")) put(name, true) })
        })
    }

    private fun restore() {
        originalBars?.let { visible ->
            controller.show(visible)
            controller.hide(WindowInsetsCompat.Type.systemBars() and visible.inv())
        }
        originalBarsBehavior?.let { controller.systemBarsBehavior = it }
        originalStyle?.let { (status, navigation) -> controller.isAppearanceLightStatusBars = status; controller.isAppearanceLightNavigationBars = navigation }
        setOrientation("restore")
        originalAwake?.let { if (it) window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON) else window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON) }
        if (originalBrightness != null) setBrightness(-1.0)
        originalBars = null; originalBarsBehavior = null; originalStyle = null; originalAwake = null
    }
    @Command fun reset(invoke: Invoke) = onMain(invoke) { restore(); invoke.resolve() }
    override fun onDestroy(activity: AppCompatActivity) { restore(); webview = null }
}

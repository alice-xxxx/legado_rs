package com.xyz.foliver.media

import android.app.Activity
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.media.MediaMetadata
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import androidx.appcompat.app.AppCompatActivity
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.net.URL

@InvokeArg
class MediaState {
    var id: Long = 0
    var title = ""
    var artist = ""
    var album = ""
    var artwork = ""
    var paused = true
    var position = 0.0
    var duration = 0.0
    var rate = 1.0
    var actions: List<String> = emptyList()
}
@InvokeArg class UpdateArgs { var state = MediaState() }
@InvokeArg class ClearArgs { var id: Long = 0 }

@TauriPlugin
class SystemMediaPlugin(private val activity: Activity) : Plugin(activity) {
    private val handler = Handler(Looper.getMainLooper())
    private val manager = activity.getSystemService(Context.AUDIO_SERVICE) as AudioManager
    private var session: MediaSession? = null
    private var state = MediaState()
    private var cover: Bitmap? = null
    private var coverRequest = ""
    private var resumeAfterFocus = false
    private var hasFocus = false
    private var registered = false
    private val focus = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
        .setWillPauseWhenDucked(true)
        .setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA).setContentType(AudioAttributes.CONTENT_TYPE_SPEECH).build())
        .setOnAudioFocusChangeListener({ change ->
            when (change) {
                AudioManager.AUDIOFOCUS_GAIN -> {
                    hasFocus = true
                    if (resumeAfterFocus) { resumeAfterFocus = false; emit("play") }
                }
                AudioManager.AUDIOFOCUS_LOSS_TRANSIENT, AudioManager.AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK -> {
                    resumeAfterFocus = resumeAfterFocus || !state.paused
                    hasFocus = false
                    emit("pause")
                }
                AudioManager.AUDIOFOCUS_LOSS -> { resumeAfterFocus = false; hasFocus = false; emit("pause") }
            }
        }, handler).build()
    private val noisy = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) { command("pause") }
    }

    private fun emit(action: String, value: Double? = null) {
        val data = JSObject().put("id", state.id).put("action", action)
        if (value != null) data.put("value", value)
        trigger("action", data)
    }
    private fun command(action: String, value: Double? = null) {
        resumeAfterFocus = false
        emit(action, value)
    }

    @Command
    fun update(invoke: Invoke) {
        val next = invoke.parseArgs(UpdateArgs::class.java).state
        handler.post {
            try {
                if (next.id < state.id) { invoke.resolve(); return@post }
                if (next.id != state.id) clearSession()
                if (session == null) {
                    session = MediaSession(activity, "Foliver").apply {
                        setFlags(MediaSession.FLAG_HANDLES_MEDIA_BUTTONS or MediaSession.FLAG_HANDLES_TRANSPORT_CONTROLS)
                        setCallback(object : MediaSession.Callback() {
                            override fun onPlay() = command("play")
                            override fun onPause() = command("pause")
                            override fun onStop() = command("stop")
                            override fun onSkipToNext() = command("nexttrack")
                            override fun onSkipToPrevious() = command("previoustrack")
                            override fun onSeekTo(pos: Long) = command("seekto", pos / 1000.0)
                            override fun onFastForward() = command("seekforward", 15.0)
                            override fun onRewind() = command("seekbackward", 15.0)
                        }, handler)
                        isActive = true
                    }
                    val filter = IntentFilter(AudioManager.ACTION_AUDIO_BECOMING_NOISY)
                    if (Build.VERSION.SDK_INT >= 33) activity.registerReceiver(noisy, filter, Context.RECEIVER_NOT_EXPORTED)
                    else activity.registerReceiver(noisy, filter)
                    registered = true
                }
                val changedCover = coverRequest != next.artwork
                val metadataChanged = state.title != next.title || state.artist != next.artist || state.album != next.album
                    || state.artwork != next.artwork || state.duration != next.duration
                state = next
                if (changedCover) { cover = null; coverRequest = next.artwork }
                if (!next.paused && !hasFocus) {
                    // Android 15+ requires a visible activity or an active foreground service for focus.
                    MediaService.session = session
                    MediaService.sessionId = next.id
                    MediaService.sendAction = { command(it) }
                    activity.startForegroundService(Intent(activity, MediaService::class.java))
                    if (manager.requestAudioFocus(focus) != AudioManager.AUDIOFOCUS_REQUEST_GRANTED) {
                        throw IllegalStateException("系统未授予音频焦点")
                    }
                    hasFocus = true
                }
                if (next.paused && hasFocus && !resumeAfterFocus) {
                    manager.abandonAudioFocusRequest(focus)
                    hasFocus = false
                }
                val controls = next.actions.fold(0L) { flags, action -> flags or when (action) {
                    "play" -> PlaybackState.ACTION_PLAY
                    "pause" -> PlaybackState.ACTION_PAUSE
                    "stop" -> PlaybackState.ACTION_STOP
                    "nexttrack" -> PlaybackState.ACTION_SKIP_TO_NEXT
                    "previoustrack" -> PlaybackState.ACTION_SKIP_TO_PREVIOUS
                    "seekto" -> PlaybackState.ACTION_SEEK_TO
                    "seekforward" -> PlaybackState.ACTION_FAST_FORWARD
                    "seekbackward" -> PlaybackState.ACTION_REWIND
                    else -> 0L
                } } or PlaybackState.ACTION_PLAY_PAUSE
                session!!.setPlaybackState(PlaybackState.Builder().setActions(controls)
                    .setState(if (next.paused) PlaybackState.STATE_PAUSED else PlaybackState.STATE_PLAYING,
                        (next.position * 1000).toLong(), if (next.paused) 0f else next.rate.toFloat(), SystemClock.elapsedRealtime()).build())
                if (metadataChanged || changedCover || session!!.controller.metadata == null) setMetadata()
                if (changedCover && next.artwork.isNotEmpty()) loadCover(next.id, next.artwork)
                MediaService.refresh()
                invoke.resolve()
            } catch (error: Exception) { invoke.reject(error.message ?: "系统媒体状态同步失败") }
        }
    }

    private fun setMetadata() {
        session?.setMetadata(MediaMetadata.Builder().putString(MediaMetadata.METADATA_KEY_TITLE, state.title)
            .putString(MediaMetadata.METADATA_KEY_ARTIST, state.artist).putString(MediaMetadata.METADATA_KEY_ALBUM, state.album)
            .putLong(MediaMetadata.METADATA_KEY_DURATION, (state.duration * 1000).toLong())
            .putBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART, cover).build())
    }
    private fun loadCover(id: Long, url: String) {
        // Only artwork is downloaded here; media content never crosses this adapter.
        Thread {
            val bitmap = runCatching {
                val connection = URL(url).openConnection().apply { connectTimeout = 5000; readTimeout = 5000 }
                val bytes = connection.getInputStream().use { input ->
                    val output = java.io.ByteArrayOutputStream()
                    val buffer = ByteArray(8192)
                    while (output.size() < 4 * 1024 * 1024) {
                        val count = input.read(buffer, 0, minOf(buffer.size, 4 * 1024 * 1024 - output.size()))
                        if (count < 0) break
                        output.write(buffer, 0, count)
                    }
                    output.toByteArray()
                }
                val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
                BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
                val options = BitmapFactory.Options().apply { inSampleSize = (maxOf(bounds.outWidth, bounds.outHeight) / 512).coerceAtLeast(1) }
                BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options)
            }.getOrNull()
            handler.post {
                if (state.id == id && state.artwork == url && session != null) { cover = bitmap; setMetadata(); MediaService.refresh() }
            }
        }.start()
    }
    @Command
    fun clear(invoke: Invoke) {
        val id = invoke.parseArgs(ClearArgs::class.java).id
        handler.post { if (id == state.id) clearSession(); invoke.resolve() }
    }
    private fun clearSession() {
        MediaService.sendAction = null
        MediaService.session = null
        activity.stopService(Intent(activity, MediaService::class.java))
        if (registered) { activity.unregisterReceiver(noisy); registered = false }
        manager.abandonAudioFocusRequest(focus)
        hasFocus = false; resumeAfterFocus = false
        session?.isActive = false
        session?.release(); session = null; cover = null; coverRequest = ""
    }
    override fun onDestroy(activity: AppCompatActivity) { clearSession() }
}

package io.legado.media

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.graphics.drawable.Icon
import android.media.MediaMetadata
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.os.IBinder
import android.os.PowerManager

/** Keeps the system session visible; it neither opens nor plays media. */
class MediaService : Service() {
    companion object {
        var session: MediaSession? = null
        var sessionId: Long = 0
        var sendAction: ((String) -> Unit)? = null
        private var instance: MediaService? = null
        fun refresh() { instance?.publish() }
    }
    private var wakeLock: PowerManager.WakeLock? = null
    private var owner: MediaSession? = null
    override fun onBind(intent: Intent?): IBinder? = null
    override fun onCreate() {
        super.onCreate()
        instance = this
        getSystemService(NotificationManager::class.java).createNotificationChannel(
            NotificationChannel("media", "媒体播放", NotificationManager.IMPORTANCE_LOW))
    }
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        owner = session
        if (owner == null) { stopSelf(); return START_NOT_STICKY }
        publish()
        if (intent?.getLongExtra("session", 0) == sessionId) intent.action?.let { sendAction?.invoke(it) }
        return START_NOT_STICKY
    }
    private fun action(name: String, icon: Int, label: String): Notification.Action {
        val intent = Intent(this, MediaService::class.java).setAction(name).putExtra("session", sessionId)
        val pending = PendingIntent.getService(this, name.hashCode(), intent, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        return Notification.Action.Builder(Icon.createWithResource(this, icon), label, pending).build()
    }
    private fun publish() {
        val current = owner ?: session ?: return
        if (current !== session) { stopSelf(); return }
        val controller = current.controller
        val state = controller.playbackState
        val playing = state?.state == PlaybackState.STATE_PLAYING
        val metadata = controller.metadata
        val launch = packageManager.getLaunchIntentForPackage(packageName)?.let {
            PendingIntent.getActivity(this, 0, it, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        }
        val notification = Notification.Builder(this, "media")
            .setSmallIcon(applicationInfo.icon.takeIf { it != 0 } ?: android.R.drawable.ic_media_play)
            .setContentTitle(metadata?.getString(MediaMetadata.METADATA_KEY_TITLE))
            .setContentText(metadata?.getString(MediaMetadata.METADATA_KEY_ARTIST))
            .setSubText(metadata?.getString(MediaMetadata.METADATA_KEY_ALBUM))
            .setLargeIcon(metadata?.getBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART))
            .setContentIntent(launch).setOnlyAlertOnce(true).setOngoing(playing)
            .setCategory(Notification.CATEGORY_TRANSPORT)
            .setVisibility(Notification.VISIBILITY_PUBLIC)
        var count = 0
        if ((state?.actions ?: 0) and PlaybackState.ACTION_SKIP_TO_PREVIOUS != 0L) {
            notification.addAction(action("previoustrack", android.R.drawable.ic_media_previous, "上一章")); count++
        }
        notification.addAction(action(if (playing) "pause" else "play",
            if (playing) android.R.drawable.ic_media_pause else android.R.drawable.ic_media_play, if (playing) "暂停" else "播放")); count++
        if ((state?.actions ?: 0) and PlaybackState.ACTION_SKIP_TO_NEXT != 0L) {
            notification.addAction(action("nexttrack", android.R.drawable.ic_media_next, "下一章")); count++
        }
        notification.setStyle(Notification.MediaStyle().setMediaSession(current.sessionToken)
            .setShowActionsInCompactView(*IntArray(count) { it }))
        notification.setDeleteIntent(PendingIntent.getService(this, 0, Intent(this, MediaService::class.java).setAction("stop").putExtra("session", sessionId),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE))
        startForeground(1, notification.build())
        if (playing && wakeLock == null) {
            wakeLock = (getSystemService(Context.POWER_SERVICE) as PowerManager)
                .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "$packageName:media").apply { acquire() }
        } else if (!playing) { wakeLock?.release(); wakeLock = null }
    }
    override fun onTaskRemoved(rootIntent: Intent?) {
        if (owner?.controller?.playbackState?.state != PlaybackState.STATE_PLAYING) stopSelf()
    }
    override fun onDestroy() {
        if (instance === this) instance = null
        wakeLock?.release(); wakeLock = null
        if (owner === session) sendAction?.invoke("pause")
        super.onDestroy()
    }
}

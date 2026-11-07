package com.omnidrop.mobile

import android.app.*
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat

/**
 * Task 4A: Persistent Foreground Service displaying real-time multi-channel transfer metrics
 * (aggregate throughput in Mbps, verified chunk progress, and active transport tier).
 */
class OmniDropForegroundService : Service() {
    companion object {
        const val CHANNEL_ID = "omnidrop_multipath_channel"
        const val NOTIFICATION_ID = 48555
        const val ACTION_START = "com.omnidrop.mobile.ACTION_START_FOREGROUND"
        const val ACTION_UPDATE = "com.omnidrop.mobile.ACTION_UPDATE_METRICS"
        const val ACTION_STOP = "com.omnidrop.mobile.ACTION_STOP_FOREGROUND"
    }

    override fun onCreate() {
        super.onCreate()
        createNotificationChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_START -> {
                val notif = buildTransferNotification(
                    title = "OmniDrop Multipath Engine Active",
                    status = "Ready for zero-config P2P transfers",
                    progress = 0,
                    maxProgress = 100,
                    indeterminate = true
                )
                startForeground(NOTIFICATION_ID, notif)
            }
            ACTION_UPDATE -> {
                val speedMbps = intent.getDoubleExtra("speed_mbps", 0.0)
                val completedChunks = intent.getIntExtra("completed_chunks", 0)
                val totalChunks = intent.getIntExtra("total_chunks", 100).coerceAtLeast(1)
                val activeChannels = intent.getIntExtra("active_channels", 1)
                val tier = intent.getStringExtra("active_tier") ?: "Tier 2 (QUIC)"

                val title = String.format("Transferring at %.1f Mbps (%d channels)", speedMbps, activeChannels)
                val status = "Chunks: $completedChunks/$totalChunks • $tier"
                val notif = buildTransferNotification(
                    title = title,
                    status = status,
                    progress = completedChunks,
                    maxProgress = totalChunks,
                    indeterminate = false
                )
                val nm = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
                nm.notify(NOTIFICATION_ID, notif)
            }
            ACTION_STOP -> {
                stopForeground(STOP_FOREGROUND_REMOVE)
                stopSelf()
            }
        }
        return START_STICKY
    }

    private fun buildTransferNotification(
        title: String,
        status: String,
        progress: Int,
        maxProgress: Int,
        indeterminate: Boolean
    ): Notification {
        val launchIntent = Intent(this, MainActivity::class.java)
        val pendingIntent = PendingIntent.getActivity(
            this, 0, launchIntent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )

        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle(title)
            .setContentText(status)
            .setSmallIcon(android.R.drawable.stat_sys_upload)
            .setContentIntent(pendingIntent)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setProgress(maxProgress, progress, indeterminate)
            .build()
    }

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                CHANNEL_ID,
                "OmniDrop Multipath Transfers",
                NotificationManager.IMPORTANCE_LOW
            ).apply {
                description = "Displays real-time P2P transfer speed and active channel aggregation"
            }
            val nm = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            nm.createNotificationChannel(channel)
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null
}

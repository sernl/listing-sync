package io.teachouse.desktop

import android.annotation.SuppressLint
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat

/**
 * Keeps the phone answering the files the seller opens in a browser while
 * the screen is off.
 *
 * The answering itself is the Rust serving loop in the same process; this
 * service does no work of its own. What it buys is standing with Android: a
 * foreground service keeps the process's network through Doze and keeps it
 * from being frozen, and the partial wake lock it holds keeps the CPU awake
 * to answer. Both last exactly as long as the service, and the ongoing
 * notification says so for the whole time: "Teachouse is sharing your files".
 *
 * Type `dataSync`, which is the type Android names for moving data over the
 * network on the user's behalf. On Android 14 and later that needs
 * `FOREGROUND_SERVICE_DATA_SYNC` in the manifest; on Android 15 and later the
 * system stops a `dataSync` service after six hours in a day, and
 * [onTimeout] stops this one when it does. Some manufacturers' battery savers
 * stop a foreground service regardless. Either way the phone stops polling
 * and the console calls it offline until the seller opens the app again.
 */
class ServeService : Service() {

    companion object {
        private const val CHANNEL = "sharing"
        private const val NOTIFICATION_ID = 0x5E4E
        private const val WAKE_LOCK_TAG = "teachouse:sharing"

        fun start(context: Context) {
            ContextCompat.startForegroundService(context, Intent(context, ServeService::class.java))
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, ServeService::class.java))
        }
    }

    private var wakeLock: PowerManager.WakeLock? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        ServiceCompat.startForeground(
            this,
            NOTIFICATION_ID,
            notification(),
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
            } else {
                0
            },
        )
        holdWakeLock()
        // Not sticky: a service Android killed is restarted by nothing but
        // the seller opening the app, which is when the Rust loop asks again.
        return START_NOT_STICKY
    }

    override fun onTimeout(startId: Int, fgsType: Int) {
        stopSelf()
    }

    override fun onDestroy() {
        wakeLock?.let { if (it.isHeld) it.release() }
        wakeLock = null
        super.onDestroy()
    }

    /**
     * Without a timeout on purpose: the service's own lifetime is the bound.
     * Rust stops it ten minutes after the last use, and the lock goes in
     * [onDestroy], which also runs when Android stops the service itself.
     */
    @SuppressLint("WakelockTimeout")
    private fun holdWakeLock() {
        if (wakeLock?.isHeld == true) return
        val power = getSystemService(Context.POWER_SERVICE) as PowerManager
        wakeLock = power.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, WAKE_LOCK_TAG).apply {
            setReferenceCounted(false)
            acquire()
        }
    }

    private fun notification(): android.app.Notification {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val manager = getSystemService(NotificationManager::class.java)
            manager.createNotificationChannel(
                NotificationChannel(
                    CHANNEL,
                    "Sharing your files",
                    NotificationManager.IMPORTANCE_LOW,
                ).apply {
                    description = "Shown while your files can be opened in the console."
                },
            )
        }
        val open = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        return NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_sharing)
            .setContentTitle("Teachouse is sharing your files")
            .setContentText("Your files can be opened in the console while this is on.")
            .setContentIntent(open)
            .setOngoing(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
            .build()
    }
}

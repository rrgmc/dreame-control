package dev.dreame.control

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import uniffi.dreame_ffi.HomeStatus
import uniffi.dreame_ffi.Notice

/** The app's notifications: news about the robot, and the running job while it is followed. */
object Notifier {
    private const val ALERTS = "alerts"
    private const val JOB = "job"
    const val JOB_ID = 1
    private var nextId = 100

    /** Creates the channels, or renames them after a language change. */
    fun channels(context: Context) {
        val m = context.getSystemService(NotificationManager::class.java)
        m.createNotificationChannel(
            NotificationChannel(ALERTS, context.getString(R.string.channel_alerts_name), NotificationManager.IMPORTANCE_DEFAULT).apply {
                description = context.getString(R.string.channel_alerts_description)
            },
        )
        m.createNotificationChannel(
            NotificationChannel(JOB, context.getString(R.string.channel_job_name), NotificationManager.IMPORTANCE_LOW).apply {
                description = context.getString(R.string.channel_job_description)
            },
        )
    }

    fun allowed(context: Context): Boolean =
        Build.VERSION.SDK_INT < 33 ||
            ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED

    private fun open(context: Context): PendingIntent = PendingIntent.getActivity(
        context,
        0,
        Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    fun notice(context: Context, n: Notice) {
        if (!allowed(context)) return
        val notification = NotificationCompat.Builder(context, ALERTS)
            .setSmallIcon(R.drawable.ic_launcher_foreground)
            .setContentTitle(n.title)
            .setContentText(n.body)
            .setContentIntent(open(context))
            .setAutoCancel(true)
            .build()
        try {
            NotificationManagerCompat.from(context).notify(nextId++, notification)
        } catch (_: SecurityException) {
        }
    }

    /** The ongoing notice of a running job. */
    fun job(context: Context, s: HomeStatus?): android.app.Notification {
        val parts = buildList {
            s?.areaM2?.let { add(context.getString(R.string.area_m2, it)) }
            s?.minutes?.let { add(context.getString(R.string.minutes_short, it)) }
            s?.progress?.let { add(context.getString(R.string.progress_done, it)) }
        }
        return NotificationCompat.Builder(context, JOB)
            .setSmallIcon(R.drawable.ic_launcher_foreground)
            .setContentTitle(s?.headline ?: context.getString(R.string.job_following))
            .setContentText(parts.joinToString("  ·  ").ifEmpty { null })
            .setContentIntent(open(context))
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setSilent(true)
            .build()
    }
}

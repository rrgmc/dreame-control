package dev.dreame.control

import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationManagerCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

/**
 * Keeps the app following a running job while it is in the background, so the user hears when
 * the job ends or the robot needs help. It shows the job in an ongoing notification and stops
 * itself when the job ends. It only listens; it never sends to the robot.
 */
class FollowService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var watching: Job? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val dreame = (application as DreameApp).dreame
        // The service can run without the activity; keep the library's text in the app's language.
        Language.apply(this)
        val type = if (Build.VERSION.SDK_INT >= 29) ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC else 0
        try {
            ServiceCompat.startForeground(this, Notifier.JOB_ID, Notifier.job(this, dreame.status.value), type)
        } catch (_: Exception) {
            // Not allowed now (the app is in the background); try again next time.
            stopSelf()
            return START_NOT_STICKY
        }
        if (watching == null) {
            watching = scope.launch {
                dreame.status.collect { s ->
                    if (s == null || !s.jobActive || !dreame.prefs.notify) {
                        dreame.onServiceDone()
                        stopSelf()
                        return@collect
                    }
                    if (Notifier.allowed(this@FollowService)) {
                        try {
                            NotificationManagerCompat.from(this@FollowService).notify(Notifier.JOB_ID, Notifier.job(this@FollowService, s))
                        } catch (_: SecurityException) {
                        }
                    }
                }
            }
        }
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        scope.cancel()
        stopped()
        ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
        super.onDestroy()
    }

    companion object {
        @Volatile
        private var running = false

        /** Starts following in the background. Call it while the app is on screen. */
        fun start(context: Context) {
            if (running) return
            running = true
            try {
                ContextCompat.startForegroundService(context, Intent(context, FollowService::class.java))
            } catch (_: Exception) {
                running = false
            }
        }

        internal fun stopped() {
            running = false
        }
    }
}

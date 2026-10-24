package app.isctcompanion

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.OutOfQuotaPolicy
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** WorkManager's periodic wake while the app is out of front. Core reads
 * what it may there (LMS deadlines and assignment states over the token,
 * reference files) and runs reminders; the school web views stay closed.
 * Android picks the time; Core decides what is due. */
class BackgroundWake(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result {
        val model = (applicationContext as IsctApplication).model
        withContext(Dispatchers.Main) { model.backgroundWake() }
        return Result.success()
    }

    companion object {
        private const val NAME = "core-background-wake"

        fun schedule(context: Context) {
            val request = PeriodicWorkRequestBuilder<BackgroundWake>(30, TimeUnit.MINUTES)
                .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
                .build()
            WorkManager.getInstance(context)
                .enqueueUniquePeriodicWork(NAME, ExistingPeriodicWorkPolicy.KEEP, request)
        }

        /** One wake as soon as Android allows, network or not: the session
         * opens and the reminder alarms are scheduled again. */
        fun soon(context: Context) {
            val request = OneTimeWorkRequestBuilder<BackgroundWake>()
                .setExpedited(OutOfQuotaPolicy.RUN_AS_NON_EXPEDITED_WORK_REQUEST)
                .build()
            WorkManager.getInstance(context).enqueueUniqueWork("$NAME-now", ExistingWorkPolicy.REPLACE, request)
        }
    }
}

/** A reboot or an app update clears every alarm, which iOS's scheduled
 * notifications survive; this brings Core's reminders back without waiting
 * for the app to be opened or the next periodic wake. */
class RescheduleReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action == Intent.ACTION_BOOT_COMPLETED || intent.action == Intent.ACTION_MY_PACKAGE_REPLACED) {
            BackgroundWake.soon(context)
        }
    }
}

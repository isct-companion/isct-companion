package app.isctcompanion

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.OutOfQuotaPolicy
import androidx.work.WorkInfo
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import androidx.work.workDataOf
import app.isctcompanion.core.BackgroundWork
import app.isctcompanion.core.WakeDirective
import app.isctcompanion.design.Format
import java.time.Instant
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** One WorkManager opportunity while the app is out of front. Core's
 * directive says when work is next due and whether it needs the network;
 * the network kind reads what Core may read there (LMS deadlines and
 * assignment states over the token, reference files) and syncs the cloud,
 * the local kind plans reminders, runs the person's tasks and maintenance.
 * Android picks the time after the earliest one; Core decides what runs. */
class BackgroundWake(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result {
        val model = (applicationContext as IsctApplication).model
        val network = inputData.getBoolean(NETWORK, false)
        withContext(Dispatchers.Main) {
            model.backgroundWake(network, if (network) 120u else 60u, inputData.getBoolean(SYSTEM_CHANGE, false))
        }
        return Result.success()
    }

    companion object {
        private const val NAME = "core-background"
        private const val NETWORK = "network"
        private const val SYSTEM_CHANGE = "system-change"

        /** One unique request per kind of work in Core's directive, at its
         * earliest time; a later directive replaces it, and a kind it no
         * longer names is cancelled. Network work waits for a connection,
         * local work does not. A worker of that kind running now is never
         * replaced: the next request queues after it. */
        suspend fun schedule(context: Context, directive: WakeDirective?) = withContext(Dispatchers.IO) {
            val manager = WorkManager.getInstance(context)
            val requests = directive?.background.orEmpty()
            for (network in listOf(true, false)) {
                val name = "$NAME-${if (network) "network" else "local"}"
                val running = runCatching {
                    manager.getWorkInfosForUniqueWork(name).get().any { it.state == WorkInfo.State.RUNNING }
                }.getOrDefault(false)
                val request = requests.firstOrNull { (it.work == BackgroundWork.NETWORK) == network }
                if (request == null) {
                    if (!running) manager.cancelUniqueWork(name)
                    continue
                }
                val earliest = Format.date(request.earliest) ?: Instant.now()
                val delay = (earliest.toEpochMilli() - Instant.now().toEpochMilli()).coerceAtLeast(0)
                val constraints = Constraints.Builder()
                    .setRequiredNetworkType(if (network) NetworkType.CONNECTED else NetworkType.NOT_REQUIRED)
                    .build()
                val work = OneTimeWorkRequestBuilder<BackgroundWake>()
                    .setInitialDelay(delay, TimeUnit.MILLISECONDS)
                    .setConstraints(constraints)
                    .setInputData(workDataOf(NETWORK to network))
                    .build()
                val policy = if (running) ExistingWorkPolicy.APPEND_OR_REPLACE else ExistingWorkPolicy.REPLACE
                manager.enqueueUniqueWork(name, policy, work)
            }
        }

        /** One local opportunity as soon as Android allows: the session opens
         * and the reminder alarms are scheduled again. */
        fun soon(context: Context) {
            val request = OneTimeWorkRequestBuilder<BackgroundWake>()
                .setExpedited(OutOfQuotaPolicy.RUN_AS_NON_EXPEDITED_WORK_REQUEST)
                .setInputData(workDataOf(NETWORK to false, SYSTEM_CHANGE to true))
                .build()
            WorkManager.getInstance(context).enqueueUniqueWork("$NAME-now", ExistingWorkPolicy.REPLACE, request)
        }
    }
}

/** A reboot or an app update clears every alarm, which iOS's scheduled
 * notifications survive; this brings Core's reminders back without waiting
 * for the app to be opened or the next opportunity. */
class RescheduleReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        when (intent.action) {
            Intent.ACTION_BOOT_COMPLETED, Intent.ACTION_MY_PACKAGE_REPLACED,
            Intent.ACTION_TIME_CHANGED, Intent.ACTION_TIMEZONE_CHANGED -> BackgroundWake.soon(context)
        }
    }
}

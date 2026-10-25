package app.isctcompanion.core

import android.app.DownloadManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.net.Uri
import android.provider.Settings
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import app.isctcompanion.design.t
import java.io.File
import java.time.Instant

/**
 * Android's update from GitHub Releases (iOS updates through TestFlight and
 * the App Store). Core decides whether a check is due and whether a newer
 * release with its APK exists (`checkAndroidUpdate`); this keeps what it
 * found, downloads the APK with the system's download manager and hands it to
 * the system installer, where the person confirms. Android itself refuses an
 * APK signed by another key.
 */
class AppUpdater(private val context: Context) {
    /** A newer release to offer on 今日, kept until it is installed. */
    var available: AppRelease? by mutableStateOf(null)
        private set
    var downloading: Boolean by mutableStateOf(false)
        private set

    private val prefs = context.getSharedPreferences("app-updates", Context.MODE_PRIVATE)
    private val running: String =
        context.packageManager.getPackageInfo(context.packageName, 0).versionName ?: "0.0.0"

    init {
        // What an earlier check found, unless this is that version now.
        val version = prefs.getString("version", null)
        val url = prefs.getString("url", null)
        if (version != null && url != null && version != running) available = AppRelease(version, url)
    }

    /** Asks Core when a check is due; `manual` checks now. */
    suspend fun check(session: Session, manual: Boolean = false) {
        val result = runCatching {
            session.checkAndroidUpdate(running, prefs.getString("lastChecked", null), manual).value()
        }.getOrNull() as? OperationResult.AppUpdate ?: return
        if (!result.checked) return
        available = result.release
        prefs.edit()
            .putString("lastChecked", Instant.now().toString())
            .putString("version", result.release?.version)
            .putString("url", result.release?.url)
            .apply()
    }

    /** Downloads the APK, then opens the system installer on it. */
    fun install() {
        val release = available ?: return
        // The system asks once whether this app may install packages.
        if (!context.packageManager.canRequestPackageInstalls()) {
            context.startActivity(
                Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${context.packageName}"))
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            )
            return
        }
        val name = "IsctCompanion_${release.version}.apk"
        val file = File(context.getExternalFilesDir("updates"), name)
        file.delete()
        val downloads = context.getSystemService(DownloadManager::class.java)
        val id = downloads.enqueue(
            DownloadManager.Request(Uri.parse(release.url))
                .setTitle(t("IsctCompanion {{version}}", "version" to release.version))
                .setMimeType(APK)
                .setDestinationInExternalFilesDir(context, "updates", name)
                .setNotificationVisibility(DownloadManager.Request.VISIBILITY_VISIBLE_NOTIFY_COMPLETED),
        )
        downloading = true
        ContextCompat.registerReceiver(
            context,
            object : BroadcastReceiver() {
                override fun onReceive(receiver: Context, intent: Intent) {
                    if (intent.getLongExtra(DownloadManager.EXTRA_DOWNLOAD_ID, -1) != id) return
                    context.unregisterReceiver(this)
                    downloading = false
                    if (succeeded(downloads, id) && file.exists()) open(file)
                }
            },
            IntentFilter(DownloadManager.ACTION_DOWNLOAD_COMPLETE),
            ContextCompat.RECEIVER_EXPORTED,
        )
    }

    private fun succeeded(downloads: DownloadManager, id: Long): Boolean =
        downloads.query(DownloadManager.Query().setFilterById(id)).use { cursor ->
            cursor.moveToFirst() &&
                cursor.getInt(cursor.getColumnIndexOrThrow(DownloadManager.COLUMN_STATUS)) == DownloadManager.STATUS_SUCCESSFUL
        }

    private fun open(file: File) {
        val uri = FileProvider.getUriForFile(context, "${context.packageName}.files", file)
        context.startActivity(
            Intent(Intent.ACTION_VIEW)
                .setDataAndType(uri, APK)
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK),
        )
    }

    private companion object {
        const val APK = "application/vnd.android.package-archive"
    }
}

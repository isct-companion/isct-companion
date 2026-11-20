package app.isctcompanion.core

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CancellationException

/** The cloud journal and the backup operations that update it. */
internal class CloudBackup(
    private val session: () -> Session?,
    private val refreshWorkspace: suspend () -> Unit,
    private val reportFailure: (Throwable) -> Unit,
) {
    var state: SyncState? by mutableStateOf(null)
        private set

    suspend fun refresh() {
        val session = session() ?: return
        (runCatching { session.cloudState().value() }.getOrNull() as? OperationResult.Cloud)
            ?.let { state = it.state }
    }

    /** Turning Drive on first shows Google's consent; Core keeps it off with
     * the reason if that fails. */
    suspend fun setEnabled(provider: CloudProvider, enabled: Boolean) =
        cloudOperation { it.setCloudEnabled(provider, enabled) }

    suspend fun setAutomatic(automatic: Boolean) = cloudOperation { it.setCloudAutomatic(automatic) }

    suspend fun sync(direction: Direction) {
        cloudOperation { it.syncCloud(direction) }
        refreshWorkspace()
    }

    /** Restores a cloud version or a local recovery file, on every connected device. */
    suspend fun restore(id: String) {
        cloudOperation { it.restoreCloud(id) }
        refreshWorkspace()
    }

    /** The current workspace as a backup file (the desktop's Export…). */
    suspend fun export(): String? {
        val session = session() ?: return null
        return try {
            (session.exportBackup().value() as? OperationResult.Text)?.value
        } catch (error: Exception) {
            reportFailure(error)
            null
        }
    }

    /** Imports a backup file; Core saves a recovery file first and publishes
     * the import like any restore. Core rejects anything that is not a backup. */
    suspend fun importBackup(text: String) {
        val session = session() ?: return
        session.applyBackup(text).value()
        refresh()
        refreshWorkspace()
    }

    /** Whether this device holds the school-login backup's password; null
     * without a session or when the vault cannot be read. */
    suspend fun credentialBackupConfigured(): Boolean? = flag { it.credentialBackupConfigured() }

    /** Whether Drive holds a school-login backup; null when Drive cannot be
     * reached, as the desktop shows "Checking Google Drive…". */
    suspend fun credentialBackupExists(): Boolean? = flag { it.credentialBackupExists() }

    /** Core restores the Drive backup with [password] on a device without
     * one, otherwise encrypts this device's logins under it. Throws Core's
     * reason (a wrong password, a short new one). */
    suspend fun enableCredentialBackup(password: String) {
        session()?.enableCredentialBackup(password)?.value()
    }

    /** Forgets this device's password; the Drive file stays for other devices. */
    suspend fun disableCredentialBackup() {
        session()?.disableCredentialBackup()?.value()
    }

    private suspend fun flag(start: (Session) -> Operation): Boolean? {
        val session = session() ?: return null
        return try {
            (start(session).value() as? OperationResult.Flag)?.value
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            null
        }
    }

    private suspend fun cloudOperation(start: (Session) -> Operation) {
        val session = session() ?: return
        try {
            (start(session).value() as? OperationResult.Cloud)?.let { state = it.state }
        } catch (error: Exception) {
            reportFailure(error)
            refresh()
        }
    }

    /** After a full reset, as the desktop's: the providers that were on are
     * turned on again, then their copy is pulled. Throws when either fails. */
    suspend fun restoreAfterReset(providers: List<CloudProvider>) {
        val session = session() ?: return
        for (provider in providers) session.setCloudEnabled(provider, true).value()
        session.syncCloud(Direction.PULL).value()
        refresh()
        refreshWorkspace()
    }
}

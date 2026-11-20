package app.isctcompanion.core

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue

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

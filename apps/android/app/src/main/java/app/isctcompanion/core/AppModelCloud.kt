package app.isctcompanion.core

import app.isctcompanion.core.AppModel.Companion.describe

// Backup: the desktop's Backup settings over the same Core journal. Core
// decides what uploads, merges and restores.

internal suspend fun AppModel.refreshCloud() {
    val session = session ?: return
    (runCatching { session.cloudState().value() }.getOrNull() as? OperationResult.Cloud)
        ?.let { cloud = it.state }
}

/** Turning Drive on first shows Google's consent; Core keeps it off with
 * the reason if that fails. */
internal suspend fun AppModel.setCloudEnabled(provider: CloudProvider, enabled: Boolean) =
    cloudOperation { it.setCloudEnabled(provider, enabled) }

internal suspend fun AppModel.setCloudAutomatic(automatic: Boolean) = cloudOperation { it.setCloudAutomatic(automatic) }

internal suspend fun AppModel.syncCloud(direction: Direction) {
    cloudOperation { it.syncCloud(direction) }
    refresh()
}

/** Restores a cloud version or a local recovery file, on every connected device. */
internal suspend fun AppModel.restoreCloud(id: String) {
    cloudOperation { it.restoreCloud(id) }
    refresh()
}

/** The current workspace as a backup file (the desktop's Export…). */
internal suspend fun AppModel.exportBackup(): String? {
    val session = session ?: return null
    return try {
        (session.exportBackup().value() as? OperationResult.Text)?.value
    } catch (error: Exception) {
        problem = describe(error)
        null
    }
}

/** Imports a backup file; Core saves a recovery file first and publishes
 * the import like any restore. Core rejects anything that is not a backup. */
internal suspend fun AppModel.importBackup(text: String) {
    val session = session ?: return
    session.applyBackup(text).value()
    refreshCloud()
    refresh()
}

private suspend fun AppModel.cloudOperation(start: (Session) -> Operation) {
    val session = session ?: return
    try {
        (start(session).value() as? OperationResult.Cloud)?.let { cloud = it.state }
    } catch (error: Exception) {
        problem = describe(error)
        refreshCloud()
    }
}

package app.isctcompanion.core

import app.isctcompanion.design.t
import kotlinx.coroutines.CancellationException
import app.isctcompanion.core.AppModel.Companion.describe

// Hand-in (slotledger `approach/share-entry-points`): files go to the assignment's
// workspace `submit/`; handing in fills the LMS draft area from it. Core
// never submits; the person saves the page.

/** The files waiting in the assignment's `submit/`. */
internal suspend fun AppModel.submitFiles(assignment: String): List<LocalFile> {
    val session = session ?: return emptyList()
    val result = runCatching { session.assignmentSubmitFiles(assignment).value() }.getOrNull()
    return (result as? OperationResult.Files)?.files.orEmpty()
}

/** Copies the files into the assignment's `submit/`; the sources stay. */
internal suspend fun AppModel.addSubmitFiles(assignment: String, files: List<SharedFile>): List<LocalFile> {
    val session = session ?: throw SdkException.Closed()
    val listed = (session.addAssignmentFiles(assignment, files).value() as? OperationResult.Files)?.files.orEmpty()
    refreshWaiting()
    return listed
}

internal suspend fun AppModel.removeSubmitFile(assignment: String, name: String): List<LocalFile> {
    val session = session ?: throw SdkException.Closed()
    val listed = (session.removeAssignmentFile(assignment, name).value() as? OperationResult.Files)?.files.orEmpty()
    refreshWaiting()
    return listed
}

/** Re-reads which assignments have files waiting to be handed in. */
internal suspend fun AppModel.refreshWaiting() {
    val session = session ?: return
    val result = runCatching { session.waitingHandIns().value() }.getOrNull() as? OperationResult.Waiting ?: return
    waiting = result.assignments.groupBy({ it.assignmentId }, { it.files.toInt() }).mapValues { it.value.sum() }
}

/** Queues `submit/` for the LMS draft area; returns the page that receives it. */
internal suspend fun AppModel.handIn(assignment: String): String {
    val session = session ?: throw SdkException.Closed()
    val messages = UploadMessages(
        t("failure.preparingYourLMSUpload"),
        t("failure.filesSentToTheUpload"),
        t("failure.theUploadAreaIsNot"),
    )
    return (session.handInAssignment(assignment, messages).value() as? OperationResult.Text)?.value
        ?: throw SdkException.Unavailable("")
}

/** Gives inbox items to an assignment and removes them; on failure they stay. */
internal suspend fun AppModel.give(items: List<SharedInbox.Item>, assignment: String): Boolean {
    val files = items.map { SharedFile(it.file.path, it.name) }
    if (files.isEmpty()) return false
    return try {
        addSubmitFiles(assignment, files)
        items.forEach { SharedInbox.remove(context, it) }
        true
    } catch (error: CancellationException) {
        throw error
    } catch (error: Exception) {
        problem = describe(error)
        false
    }
}

package app.isctcompanion.core

import app.isctcompanion.design.t

/** Queues this selection for the LMS draft page the person will save. */
internal suspend fun AppModel.handIn(
    selection: SharedInbox.Selection,
    assignmentId: String,
): OperationResult.DraftUpload {
    val session = session ?: throw SdkException.Closed()
    val messages = UploadMessages(
        t("failure.preparingYourLMSUpload"),
        t("failure.filesSentToTheUpload"),
        t("failure.theUploadAreaIsNot"),
    )
    val files = selection.files.map { SharedFile(it.file.path, it.name) }
    return session.prepareHandIn(assignmentId, files, messages).value() as? OperationResult.DraftUpload
        ?: throw SdkException.Unavailable("")
}

import IsctCore

extension AppModel {
    /// Queues this selection for the LMS draft page the person will save.
    func handIn(_ assignment: String, title: String, files: [SharedFile]) async throws -> WebRouter.PushedPage {
        guard let session else { throw SdkError.Closed }
        let messages = UploadMessages(
            waiting: t("failure.preparingYourLMSUpload"),
            queued: t("failure.filesSentToTheUpload"),
            failed: t("failure.theUploadAreaIsNot"))
        guard case let .draftUpload(id, url) = try await session.prepareHandIn(
            assignmentId: assignment, files: files, messages: messages).value()
        else { throw SdkError.Unavailable(detail: "") }
        return WebRouter.PushedPage(site: .scienceLms, target: url, title: title, uploadId: id)
    }

    /// The quarter's current draft targets, for the share extension.
    func writeShareChoices(_ board: AssignmentBoard) {
        let groups = board.today
        let rows = groups.pinned + groups.dueThisWeek + groups.later + groups.undated + groups.pastRecent
        var seen = Set<String>()
        let choices = rows.filter { $0.handInAvailability == .prepareDraft && seen.insert($0.assignmentId).inserted }.map {
            SharedInbox.Choice(id: $0.assignmentId, title: $0.title, course: $0.courseTitle, due: Format.due($0.dueAt))
        }
        SharedInbox.writeChoices(SharedInbox.Choices(language: Localizer.shared.language, assignments: choices))
    }
}

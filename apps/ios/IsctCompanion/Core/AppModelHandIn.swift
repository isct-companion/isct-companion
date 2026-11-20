import IsctCore

// MARK: - Hand-in (see HandIn.swift)

extension AppModel {
    /// The files waiting in the assignment's `submit/`.
    func submitFiles(_ assignment: String) async -> [LocalFile] {
        guard let session,
            case let .files(files)? = try? await session.assignmentSubmitFiles(assignmentId: assignment).value()
        else { return [] }
        return files
    }

    /// Copies the files into the assignment's `submit/`; the sources stay.
    func addSubmitFiles(_ assignment: String, _ files: [SharedFile]) async throws -> [LocalFile] {
        guard let session else { throw SdkError.Closed }
        guard case let .files(listed) = try await session.addAssignmentFiles(assignmentId: assignment, files: files).value()
        else { return [] }
        await refreshWaiting()
        return listed
    }

    func removeSubmitFile(_ assignment: String, name: String) async throws -> [LocalFile] {
        guard let session else { throw SdkError.Closed }
        guard case let .files(listed) = try await session.removeAssignmentFile(assignmentId: assignment, name: name).value()
        else { return [] }
        await refreshWaiting()
        return listed
    }

    /// Re-reads which assignments have files waiting to be handed in.
    func refreshWaiting() async {
        guard let session,
            case let .waiting(assignments)? = try? await session.waitingHandIns().value()
        else { return }
        waiting = Dictionary(assignments.map { ($0.assignmentId, Int($0.files)) }, uniquingKeysWith: +)
    }

    /// Queues `submit/` for the LMS draft area and returns the page that
    /// receives it, for the in-app web view.
    func handIn(_ assignment: String) async throws -> String {
        guard let session else { throw SdkError.Closed }
        let messages = UploadMessages(
            waiting: t("failure.preparingYourLMSUpload"),
            queued: t("failure.filesSentToTheUpload"),
            failed: t("failure.theUploadAreaIsNot"))
        guard case let .text(url) = try await session.handInAssignment(assignmentId: assignment, messages: messages).value()
        else { throw SdkError.Unavailable(detail: "") }
        return url
    }

    /// The quarter's open assignments, for the share sheet's list.
    func writeShareChoices(_ board: AssignmentBoard) {
        let groups = board.today
        let open = groups.pinned + groups.dueThisWeek + groups.later + groups.undated + groups.pastRecent
        var seen = Set<String>()
        let choices = open.filter { seen.insert($0.assignmentId).inserted }.map {
            SharedInbox.Choice(id: $0.assignmentId, title: $0.title, course: $0.courseTitle, due: Format.due($0.dueAt))
        }
        SharedInbox.writeChoices(SharedInbox.Choices(language: Localizer.shared.language, assignments: choices))
    }
}

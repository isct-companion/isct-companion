import CoreGraphics
import Foundation
import IsctCore

// The phones' hand-in (slotledger `approach/share-entry-points`): what was shared into the
// app waits in `SharedInbox`; the person gives it to an assignment, Core puts
// it in that assignment's workspace `submit/`, and handing in fills the LMS
// draft area from there. Core never submits; the person saves the page.

/// What waits in the shared inbox, for the corner thumbnails and the inbox
/// sheet. Reads the App Group folder when the app comes forward.
@MainActor
@Observable
final class InboxModel {
    private(set) var items: [SharedInbox.Item] = []
    /// The assignment a share-sheet pick was just handed to, for the app to show.
    var handedTo: String?
    /// The upload panel is up (the corner stack was tapped).
    var presenting = false
    var error: String?

    /// Where the corner stack can be dropped: assignment rows and upload
    /// cards, by a token each view keeps. Read only while dragging.
    struct Target {
        let assignment: String
        let frame: CGRect
        /// A row opens its assignment after the drop; a page's own card is
        /// already open.
        let opens: Bool
    }
    @ObservationIgnored var targets: [UUID: Target] = [:]
    /// The target under the dragged stack, which floats.
    private(set) var hovered: UUID?

    /// Finds the target under a point of the window while the stack is dragged.
    func hover(at point: CGPoint?) {
        let next = point.flatMap { point in targets.first { $0.value.frame.contains(point) }?.key }
        if next != hovered { hovered = next }
    }

    var hoveredTarget: Target? { hovered.flatMap { targets[$0] } }

    /// Removes everything waiting, thrown into the corner's trash.
    func discardAll() {
        items.forEach(SharedInbox.remove)
        items = SharedInbox.items()
    }

    /// A file another app opened in this one ("Open in IsctCompanion"): the
    /// system's copy joins the corner stack, as a file chosen in the app would.
    func receive(_ url: URL) {
        guard url.isFileURL else { return }
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        do {
            try SharedInbox.add(url, name: url.lastPathComponent, assignmentId: nil)
            // The system put its copy in Documents/Inbox; the inbox keeps its own.
            try? FileManager.default.removeItem(at: url)
            items = SharedInbox.items()
        } catch {
            self.error = AppModel.describe(error)
        }
    }

    func refresh(_ model: AppModel) async {
        items = SharedInbox.items()
        // Files given an assignment in the share sheet go straight there.
        let picked = Dictionary(grouping: items.filter { $0.assignmentId != nil }) { $0.assignmentId ?? "" }
        for (assignment, items) in picked {
            if await give(items, to: assignment, model) { handedTo = assignment }
        }
    }

    /// Hands the items to the assignment's workspace and removes them from the
    /// inbox; on failure they stay for another try.
    @discardableResult
    func give(_ items: [SharedInbox.Item], to assignment: String, _ model: AppModel) async -> Bool {
        let files = items.compactMap { item in item.url.map { SharedFile(path: $0.path, name: item.name) } }
        guard !files.isEmpty else { return false }
        do {
            _ = try await model.addSubmitFiles(assignment, files)
            items.forEach(SharedInbox.remove)
            self.items = SharedInbox.items()
            return true
        } catch {
            self.error = AppModel.describe(error)
            return false
        }
    }

    func remove(_ item: SharedInbox.Item) {
        SharedInbox.remove(item)
        items = SharedInbox.items()
    }

    func items(_ ids: [String]) -> [SharedInbox.Item] {
        items.filter { ids.contains($0.id) }
    }
}

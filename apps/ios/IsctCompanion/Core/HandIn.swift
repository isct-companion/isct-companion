import Foundation
import IsctCore

/// Holds only the current selection. A share prepared by the extension is
/// adopted once when the app opens.
@MainActor
@Observable
final class InboxModel {
    private(set) var transfer: SharedInbox.Transfer?
    private(set) var working = false
    private var generation = 0
    var error: String?

    func refresh() {
        if transfer == nil, let shared = SharedInbox.take() {
            adopt(shared, generation: nextGeneration())
        }
    }

    func discard(_ selection: SharedInbox.Transfer) {
        guard transfer?.id == selection.id else { return }
        _ = nextGeneration()
        transfer = nil
        selection.discard()
    }

    func give(
        _ selection: SharedInbox.Transfer, to assignment: String, title: String, model: AppModel
    ) async throws -> WebRouter.PushedPage? {
        guard !working, transfer?.id == selection.id else { return nil }
        working = true
        defer { working = false }
        let files = selection.files.map { SharedFile(path: $0.url(in: selection.folder).path, name: $0.name) }
        let page = try await model.handIn(assignment, title: title, files: files)
        guard transfer?.id == selection.id else {
            if let uploadId = page.uploadId { model.web.cancelHandIn(uploadId) }
            return nil
        }
        discard(selection)
        return page
    }

    func select(
        _ choice: SharedInbox.Choice?,
        copying: (URL) async throws -> [SharedInbox.File]
    ) async {
        let current = nextGeneration()
        do {
            let folder = try SharedInbox.temporaryFolder()
            finish(.success(try await copy(copying, into: folder)), folder: folder, choice: choice, generation: current)
        } catch {
            if generation == current { self.error = AppModel.describe(error) }
        }
    }

    private func copy(
        _ copying: (URL) async throws -> [SharedInbox.File], into folder: URL
    ) async throws -> [SharedInbox.File] {
        do {
            let files = try await copying(folder)
            try Task.checkCancellation()
            return files
        } catch {
            try? FileManager.default.removeItem(at: folder)
            throw error
        }
    }

    func receive(_ urls: [URL], choice: SharedInbox.Choice? = nil) async {
        await select(choice) { folder in
            try urls.map { url in
                let scoped = url.startAccessingSecurityScopedResource()
                defer { if scoped { url.stopAccessingSecurityScopedResource() } }
                return try SharedInbox.copy(url, name: url.lastPathComponent, into: folder)
            }
        }
    }

    /// Open In supplies an app-owned Documents/Inbox copy. A picker or a
    /// drop may supply the user's source URL and does not use this entry.
    func receiveOpenIn(_ url: URL) async {
        defer {
            let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first
            let inbox = documents?.appendingPathComponent("Inbox", isDirectory: true)
                .resolvingSymlinksInPath().path
            if url.deletingLastPathComponent().resolvingSymlinksInPath().path == inbox {
                try? FileManager.default.removeItem(at: url)
            }
        }
        await receive([url])
    }

    /// Starts each provider synchronously inside SwiftUI's drop callback.
    func receive(_ providers: [NSItemProvider], choice: SharedInbox.Choice) -> Bool {
        let current = nextGeneration()
        do {
            let folder = try SharedInbox.temporaryFolder()
            SharedInbox.startCopy(providers, into: folder) { result in
                self.finish(result, folder: folder, choice: choice, generation: current)
            }
            return true
        } catch {
            self.error = AppModel.describe(error)
            return false
        }
    }

    private func nextGeneration() -> Int {
        generation += 1
        return generation
    }

    private func finish(
        _ result: Result<[SharedInbox.File], Error>, folder: URL,
        choice: SharedInbox.Choice?, generation: Int
    ) {
        do {
            let files = try result.get()
            if !files.isEmpty {
                adopt(SharedInbox.Transfer(folder: folder, files: files, choice: choice), generation: generation)
                return
            }
        } catch {
            if self.generation == generation { self.error = AppModel.describe(error) }
        }
        try? FileManager.default.removeItem(at: folder)
    }

    private func adopt(_ selection: SharedInbox.Transfer, generation: Int) {
        guard self.generation == generation else { selection.discard(); return }
        transfer?.discard()
        transfer = selection
    }
}

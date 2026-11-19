import IsctCore
import QuickLook
import SwiftUI

/// One file presentation per list. Quick Look and Files receive named copies
/// of the version Core opened, while Core keeps the original for offline use.
@MainActor
@Observable
final class ExamDocumentOpening {
    private(set) var opening: String?
    private(set) var previewURL: URL?
    var downloadURL: URL?
    var reportID: String?
    var uploadedID: String?
    private var opened: ExamLocalDocument?
    private var generation = 0
    private(set) var savedByID: [String: Bool] = [:]

    func document(_ id: String) -> ExamLocalDocument? {
        opened?.document.id == id ? opened : nil
    }

    func preview(_ id: String, model: AppModel, page: ExamPage) async {
        _ = await prepare(id, for: .preview, model: model, page: page)
    }

    func download(_ id: String, model: AppModel, page: ExamPage) async -> Bool {
        await prepare(id, for: .download, model: model, page: page)
    }

    func setSaved(_ id: String, _ saved: Bool, model: AppModel, page: ExamPage) async -> Bool {
        let session = generation
        guard case let .saved(value)? = await page.call(model, .save(id: id, saved: saved)),
              isCurrent(session, model: model) else { return false }
        savedByID[id] = value
        page.notice = value ? t("examShare.saved") : nil
        return true
    }

    func previewUploadAfterDismissal(model: AppModel, page: ExamPage) {
        guard let uploadedID else { return }
        self.uploadedID = nil
        Task { await preview(uploadedID, model: model, page: page) }
    }

    func forget(_ id: String) {
        if opened?.document.id == id { opened = nil }
    }

    func dismissPreview() {
        guard let previewURL else { return }
        self.previewURL = nil
        removeCopy(previewURL)
    }

    func clear() {
        generation += 1
        opening = nil
        opened = nil
        savedByID.removeAll()
        downloadURL = nil
        reportID = nil
        uploadedID = nil
        dismissPreview()
    }

    private enum Destination: Equatable { case preview, download }

    private func isCurrent(_ session: Int, model: AppModel) -> Bool {
        session == generation && model.examAccount?.status == .connected
    }

    private func prepare(_ id: String, for destination: Destination, model: AppModel, page: ExamPage) async -> Bool {
        guard opening == nil else { return false }
        let session = generation
        opening = id
        defer { if session == generation { opening = nil } }

        var file: ExamLocalDocument
        if destination == .download, let lastOpened = document(id) {
            file = lastOpened
        } else {
            guard case let .document(current)? = await page.call(model, .open(id: id)),
                  isCurrent(session, model: model) else { return false }
            file = current
            opened = current
            savedByID[id] = current.saved
        }

        let needsRetention = destination == .download && !file.downloaded && !file.offline
        if needsRetention {
            let result = await page.call(model, .retainOpen(id: id, path: file.path))
            guard isCurrent(session, model: model) else { return false }
            guard case .done? = result else {
                forget(id)
                return false
            }
            file.downloaded = true
            opened = file
        }

        do {
            let copy = try await namedCopy(file)
            guard isCurrent(session, model: model) else {
                removeCopy(copy)
                return false
            }
            switch destination {
            case .preview: previewURL = copy
            case .download: downloadURL = copy
            }
        } catch {
            guard isCurrent(session, model: model) else { return false }
            forget(id)
            page.problem = t(destination == .preview ? "share.copyFailed" : "lmsFile.theFileCouldNotBeDownloaded")
        }
        return needsRetention
    }

    private func removeCopy(_ url: URL) {
        try? FileManager.default.removeItem(at: url.deletingLastPathComponent())
    }

    private func namedCopy(_ file: ExamLocalDocument) async throws -> URL {
        let source = URL(fileURLWithPath: file.path)
        let name = file.exportName
        return try await Task.detached(priority: .userInitiated) {
            let manager = FileManager.default
            let folder = manager.temporaryDirectory
                .appendingPathComponent("exam-export", isDirectory: true)
                .appendingPathComponent(UUID().uuidString, isDirectory: true)
            let copy = folder.appendingPathComponent(name)
            try manager.createDirectory(at: folder, withIntermediateDirectories: true)
            do {
                try manager.copyItem(at: source, to: copy)
                return copy
            } catch {
                try? manager.removeItem(at: folder)
                throw error
            }
        }.value
    }
}

struct ExamDocumentMenu: View {
    @Environment(AppModel.self) private var model
    let id: String
    let documents: ExamDocumentOpening
    let page: ExamPage
    var saved: Bool? = nil
    var offline = false
    var removal: ExamRemoval? = nil
    var libraryChanged: (ExamLibrary?) async -> Void = { _ in }

    var body: some View {
        let file = documents.document(id)
        let isOffline = offline || file?.offline == true
        Button(t("examShare.download"), systemImage: "arrow.down.circle") {
            Task {
                if await documents.download(id, model: model, page: page) {
                    let exportProblem = page.problem
                    await libraryChanged(nil)
                    if let exportProblem { page.problem = exportProblem }
                }
            }
        }
        .disabled(documents.opening != nil)
        Group {
            if let isSaved = saved ?? documents.savedByID[id] {
                bookmarkAction(!isSaved)
            } else {
                Menu {
                    bookmarkAction(true)
                    bookmarkAction(false)
                } label: {
                    Label(t("examShare.bookmarkActions"), systemImage: "bookmark")
                }
            }
        }
        .disabled(isOffline)
        Button(t("examShare.report"), systemImage: "flag", role: .destructive) { documents.reportID = id }
            .disabled(isOffline || page.choices == nil)
        if let web = file.flatMap({ URL(string: $0.webUrl) }) {
            Link(destination: web) { Label(t("examShare.openOnWeb"), systemImage: "safari") }
        }
        if let removal = removal ?? (file?.downloaded == true ? .download : nil) {
            Button(t(removal.key), systemImage: "trash", role: .destructive) {
                page.confirm(removal.key) {
                    if case let .library(library)? = await page.call(model, removal.action(id)) {
                        documents.forget(id)
                        await libraryChanged(library)
                    }
                }
            }
        }
    }

    private func bookmarkAction(_ save: Bool) -> some View {
        Button(t(save ? "examShare.save" : "examShare.unsave"), systemImage: save ? "bookmark" : "bookmark.slash") {
            Task {
                if await documents.setSaved(id, save, model: model, page: page) { await libraryChanged(nil) }
            }
        }
    }
}

extension View {
    func examDocumentPresentation(_ documents: ExamDocumentOpening, page: ExamPage) -> some View {
        modifier(ExamDocumentPresentation(documents: documents, page: page))
    }
}

private struct ExamDocumentPresentation: ViewModifier {
    @Environment(AppModel.self) private var model
    let documents: ExamDocumentOpening
    let page: ExamPage

    func body(content: Content) -> some View {
        content
            .quickLookPreview(Binding(get: { documents.previewURL }, set: { if $0 == nil { documents.dismissPreview() } }))
            .fileMover(isPresented: Binding(get: { documents.downloadURL != nil }, set: { if !$0 { documents.downloadURL = nil } }),
                       file: documents.downloadURL) { result in
                switch result {
                case let .success(file):
                    page.notice = t("app.savedName", ["name": file.lastPathComponent])
                case .failure:
                    page.problem = t("lmsFile.theFileCouldNotBeDownloaded")
                }
            }
            .sheet(isPresented: Binding(get: { documents.reportID != nil }, set: { if !$0 { documents.reportID = nil } })) {
                if let id = documents.reportID, let choices = page.choices {
                    ExamReportForm(reasons: choices.reportReasons, maxDetail: choices.limits.reportDetail,
                                   busy: page.busy, problem: page.problem) { reason, detail in
                        if await page.call(model, .report(id: id, reason: reason, detail: detail)) != nil {
                            documents.reportID = nil
                            page.notice = t("examShare.reported")
                        }
                    }
                }
            }
            .onChange(of: model.examAccount?.status) { _, status in
                if status != .connected { documents.clear() }
            }
    }
}

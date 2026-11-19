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
    private(set) var savedByID: [String: Bool] = [:]

    func document(_ id: String) -> ExamLocalDocument? {
        opened?.document.id == id ? opened : nil
    }

    func preview(_ id: String, model: AppModel, page: ExamPage) async {
        guard opening == nil else { return }
        opening = id
        defer { opening = nil }
        guard let file = await read(id, model: model, page: page) else { return }
        do {
            previewURL = try await namedCopy(file)
        } catch {
            page.problem = t("share.copyFailed")
        }
    }

    func download(_ id: String, model: AppModel, page: ExamPage) async -> Bool {
        guard opening == nil else { return false }
        opening = id
        defer { opening = nil }
        // Export the same version the user just saw, including an offline copy.
        let file = if let file = document(id) { file } else { await read(id, model: model, page: page) }
        guard var file else { return false }
        let needsRetention = !file.downloaded && !file.offline
        if needsRetention {
            guard case .done? = await page.call(model, .retainOpen(id: id, path: file.path)) else { return false }
            file.downloaded = true
            opened = file
        }
        do {
            downloadURL = try await namedCopy(file)
        } catch {
            forget(id)
            page.problem = t("lmsFile.theFileCouldNotBeDownloaded")
        }
        return needsRetention
    }

    func setSaved(_ id: String, _ saved: Bool, model: AppModel, page: ExamPage) async -> Bool {
        guard case let .saved(value)? = await page.call(model, .save(id: id, saved: saved)) else { return false }
        savedByID[id] = value
        if value { page.notice = t("examShare.saved") }
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
        try? FileManager.default.removeItem(at: previewURL.deletingLastPathComponent())
    }

    func clear() {
        opened = nil
        savedByID.removeAll()
        downloadURL = nil
        reportID = nil
        uploadedID = nil
        dismissPreview()
    }

    private func read(_ id: String, model: AppModel, page: ExamPage) async -> ExamLocalDocument? {
        guard case let .document(file)? = await page.call(model, .open(id: id)) else { return nil }
        opened = file
        savedByID[id] = file.saved
        return file
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
        let isSaved = saved ?? documents.savedByID[id] ?? false
        let isOffline = offline || file?.offline == true
        Button(t("examShare.download"), systemImage: "arrow.down.circle") {
            Task {
                if await documents.download(id, model: model, page: page) { await libraryChanged(nil) }
            }
        }
        .disabled(documents.opening != nil)
        Button(t(isSaved ? "examShare.unsave" : "examShare.save"), systemImage: isSaved ? "bookmark.fill" : "bookmark") {
            Task {
                if await documents.setSaved(id, !isSaved, model: model, page: page) { await libraryChanged(nil) }
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

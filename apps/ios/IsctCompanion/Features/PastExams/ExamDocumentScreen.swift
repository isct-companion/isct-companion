import IsctCore
import SwiftUI

/// A past exam full screen in Quick Look inside the page, which reads PDFs
/// and photos with its own copy and search, or a Markdown one in
/// `MarkdownView`; the document's actions sit in the toolbar's menu.
struct ExamDocumentScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let route: ExamDocumentRoute
    @State private var page = ExamPage()
    @State private var file: ExamLocalDocument?
    @State private var failed = false
    @State private var showsReport = false
    /// The opened copy under its export name, for Share: the cached file
    /// has a generated name.
    @State private var shareCopy: URL?

    var body: some View {
        Group {
            if let file {
                // The site stores Markdown as text/plain; Quick Look would show
                // its source, so it opens formatted, with its math set.
                if file.viewer == .markdown {
                    MarkdownFileView(path: file.path) {
                        QuickLookView(url: URL(fileURLWithPath: file.path))
                    }
                    .ignoresSafeArea(edges: .bottom)
                } else {
                    QuickLookView(url: URL(fileURLWithPath: file.path)).ignoresSafeArea(edges: .bottom)
                }
            } else if failed {
                ContentUnavailableView {
                    Label(route.file.map { examName($0.document) } ?? t("examShare.title"), systemImage: "doc")
                } actions: {
                    Button(t("common.retry")) { Task { await open() } }
                }
            } else {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .background(Theme.bg)
        .task(id: file?.path) {
            removeShareCopy()
            guard let file else { return }
            let (path, name) = (file.path, file.exportName)
            shareCopy = await Task.detached { Self.namedCopy(of: path, as: name) }.value
        }
        .onDisappear { removeShareCopy() }
        .navigationTitle(file.map { examName($0.document) } ?? "")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            if let file {
                ToolbarItem(placement: .topBarTrailing) { actions(file) }
            }
        }
        .task {
            await page.loadChoices(model)
            if file == nil {
                if let preloaded = route.file { file = preloaded } else { await open() }
            }
        }
        .sheet(isPresented: $showsReport) {
            if let file, let choices = page.choices {
                ExamReportForm(reasons: choices.reportReasons, maxDetail: choices.limits.reportDetail,
                               busy: page.busy, problem: page.problem) { reason, detail in
                    if await page.call(model, .report(id: file.document.id, reason: reason, detail: detail)) != nil {
                        showsReport = false
                        page.notice = t("examShare.reported")
                    }
                }
            }
        }
        .examPageFeedback(page)
    }

    private func removeShareCopy() {
        if let shareCopy { try? FileManager.default.removeItem(at: shareCopy.deletingLastPathComponent()) }
        shareCopy = nil
    }

    /// A copy of the file at `path` named `name`, in a folder of its own
    /// under the temporary directory.
    nonisolated private static func namedCopy(of path: String, as name: String) -> URL? {
        let manager = FileManager.default
        let folder = manager.temporaryDirectory
            .appendingPathComponent("exam-share", isDirectory: true)
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        let copy = folder.appendingPathComponent(name)
        do {
            try manager.createDirectory(at: folder, withIntermediateDirectories: true)
            try manager.copyItem(at: URL(fileURLWithPath: path), to: copy)
            return copy
        } catch {
            try? manager.removeItem(at: folder)
            return nil
        }
    }

    private func actions(_ file: ExamLocalDocument) -> some View {
        Menu {
            Button(t(file.downloaded ? "examShare.downloaded" : "examShare.download"), systemImage: "arrow.down.circle") {
                Task { await open(download: true) }
            }
            .disabled(file.downloaded || file.offline)
            if let saved = file.saved {
                Button(t(saved ? "examShare.unsave" : "examShare.save"), systemImage: saved ? "bookmark.fill" : "bookmark") {
                    Task { await save(!saved) }
                }
                .disabled(file.offline)
            }
            ShareLink(item: shareCopy ?? URL(fileURLWithPath: file.path)) {
                Label(t("common.share"), systemImage: "square.and.arrow.up")
            }
            if let web = URL(string: file.webUrl) {
                Link(destination: web) { Label(t("examShare.openOnWeb"), systemImage: "safari") }
            }
            Button(t("examShare.report"), systemImage: "flag", role: .destructive) { showsReport = true }
                .disabled(file.offline || page.choices == nil)
            if let removal {
                Button(t(removal.key), systemImage: "trash", role: .destructive) {
                    page.confirm(removal.key) {
                        if await page.call(model, removal.action(file.document.id)) != nil { dismiss() }
                    }
                }
            }
        } label: {
            Label(t("examShare.more"), systemImage: "ellipsis.circle")
        }
        .disabled(page.busy)
    }

    /// The library's removal, or removing the download of one opened elsewhere.
    private var removal: ExamRemoval? {
        route.removal ?? (file?.downloaded == true ? .download : nil)
    }

    private func open(download: Bool = false) async {
        failed = false
        if case let .document(value)? = await page.call(model, .open(id: route.id, download: download)) {
            file = value
            if download { page.notice = t("examShare.downloadedNotice") }
        } else {
            failed = file == nil
        }
    }

    private func save(_ saved: Bool) async {
        guard let current = file,
              case let .saved(updated)? = await page.call(model, .save(id: current.document.id, saved: saved)) else { return }
        var refreshed = current
        refreshed.saved = updated
        file = refreshed
    }
}

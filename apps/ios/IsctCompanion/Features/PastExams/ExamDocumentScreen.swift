import IsctCore
import QuickLook
import SwiftUI

/// A past exam full screen in Quick Look inside the page, which reads PDFs
/// and photos with its own copy and search; the document's actions sit in
/// the toolbar's menu.
struct ExamDocumentScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let route: ExamDocumentRoute
    @State private var page = ExamPage()
    @State private var file: ExamLocalDocument?
    @State private var failed = false
    @State private var showsReport = false

    var body: some View {
        Group {
            if let file {
                QuickLookView(url: URL(fileURLWithPath: file.path)).ignoresSafeArea(edges: .bottom)
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
            ShareLink(item: URL(fileURLWithPath: file.path)) {
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

/// Quick Look inside the page rather than over it, so the document keeps the
/// navigation bar and its actions.
private struct QuickLookView: UIViewControllerRepresentable {
    let url: URL

    func makeCoordinator() -> Coordinator { Coordinator(url: url) }

    func makeUIViewController(context: Context) -> QLPreviewController {
        let controller = QLPreviewController()
        controller.dataSource = context.coordinator
        return controller
    }

    func updateUIViewController(_ controller: QLPreviewController, context: Context) {
        guard context.coordinator.url != url else { return }
        context.coordinator.url = url
        controller.reloadData()
    }

    final class Coordinator: NSObject, QLPreviewControllerDataSource {
        var url: URL
        init(url: URL) { self.url = url }
        func numberOfPreviewItems(in controller: QLPreviewController) -> Int { 1 }
        func previewController(_ controller: QLPreviewController, previewItemAt index: Int) -> QLPreviewItem {
            url as NSURL
        }
    }
}

/// One request in full, then the one thing to do with it: share a file for
/// it, see the shared one, or close, reopen or delete one's own.
struct ExamRequestScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @Environment(\.push) private var push
    let route: ExamRequestRoute
    @State private var page = ExamPage()
    @State private var item: ExamRequest?
    @State private var webUrl: String?
    @State private var uploadEntry: ExamUploadEntry?

    var body: some View {
        let item = item ?? route.request
        List {
            Section {
                VStack(alignment: .leading, spacing: 6) {
                    Text(item.subject.code).font(.footnote).foregroundStyle(Theme.muted)
                    Text(item.subject.name).font(.title3.weight(.semibold)).foregroundStyle(Theme.ink)
                    ExamStatusTag(status: item.status)
                }
                let facts = examRequestFacts(item)
                if !facts.isEmpty { Text(facts).foregroundStyle(Theme.muted) }
                if !item.teachers.isEmpty {
                    Text(item.teachers.map(\.name).joined(separator: " / ")).foregroundStyle(Theme.muted)
                }
            }
            if !item.description.isEmpty {
                Section { Text(item.description).foregroundStyle(Theme.ink) }
            }
            Section {
                if let answer = item.answerExamId {
                    NavigationLink(value: ExamDocumentRoute(id: answer)) {
                        Label(t("examShare.viewAnswer"), systemImage: "doc.text")
                    }
                }
                if !item.mine && item.status == "open" {
                    Button(t("examShare.upload.answerRequest"), systemImage: "square.and.arrow.up") { uploadEntry = .request(item) }
                        .disabled(page.choices == nil)
                }
                if item.mine {
                    Button(t(item.status == "open" ? "examShare.closeRequest" : "examShare.reopenRequest")) {
                        Task { await setOpen(item, item.status != "open") }
                    }
                    Button(t("common.delete"), role: .destructive) {
                        page.confirm("examShare.deleteRequest") {
                            if await page.call(model, .deleteRequest(id: item.id, mine: true)) != nil { dismiss() }
                        }
                    }
                }
                if let web = URL(string: webUrl ?? route.webUrl) {
                    Link(destination: web) { Label(t("examShare.openOnWeb"), systemImage: "safari") }
                }
            }
        }
        .themedRows()
        .themedList()
        .disabled(page.busy)
        .navigationTitle(t("examShare.requests"))
        .navigationBarTitleDisplayMode(.inline)
        .task { await page.loadChoices(model) }
        .sheet(item: $uploadEntry) { entry in
            if let choices = page.choices {
                ExamUploadSheet(entry: entry, choices: choices) { outcome in
                    page.notice = outcome.notice
                    if let id = outcome.documentId { push(ExamDocumentRoute(id: id)) }
                }
            }
        }
        .examPageFeedback(page)
    }

    /// Core answers the person's own board; the request shows as it is there.
    private func setOpen(_ item: ExamRequest, _ open: Bool) async {
        guard case let .requests(board)? = await page.call(model, .setRequestOpen(id: item.id, open: open, mine: true))
        else { return }
        webUrl = board.webUrl
        if let changed = board.items.first(where: { $0.id == item.id }) { self.item = changed }
    }
}

import IsctCore
import QuickLook
import SwiftUI

/// A past exam full screen in QuickLook, which reads PDFs and photos with
/// its own copy and search; the document's actions sit in the toolbar.
struct ExamDocumentScreen: View {
    @Environment(ExamShareStore.self) private var store
    let route: ExamDocumentRoute
    @State private var file: ExamLocalDocument?
    @State private var failed = false
    @State private var reporting = false

    var body: some View {
        Group {
            if let file {
                QuickLookView(url: URL(fileURLWithPath: file.path)).ignoresSafeArea(edges: .bottom)
            } else if failed {
                ContentUnavailableView {
                    Label(t("examShare.file"), systemImage: "doc")
                } actions: {
                    Button(t("common.retry")) { Task { await open() } }
                }
            } else {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .background(Theme.bg)
        .navigationTitle(file.map { store.title($0.document) } ?? "")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            if let file {
                ToolbarItem(placement: .topBarTrailing) {
                    Menu {
                        Button(t(file.downloaded ? "examShare.downloaded" : "examShare.download"),
                               systemImage: "arrow.down.circle") {
                            Task { await open(download: true) }
                        }
                        .disabled(file.downloaded || file.offline)
                        if let saved = file.saved {
                            Button(t(saved ? "examShare.unsave" : "examShare.save"),
                                   systemImage: saved ? "bookmark.fill" : "bookmark") {
                                Task { await save(!saved) }
                            }
                            .disabled(file.offline)
                        }
                        if let url = ExamSite.document(file.document.id) {
                            Link(destination: url) { Label(t("examShare.openInBrowser"), systemImage: "safari") }
                        }
                        Button(t("examShare.report"), systemImage: "flag", role: .destructive) { reporting = true }
                            .disabled(file.offline)
                    } label: {
                        Label(t("examShare.report"), systemImage: "ellipsis.circle")
                    }
                }
            }
        }
        .sheet(isPresented: $reporting) {
            if let file {
                ExamReportForm { reason, detail in
                    if await store.call(.report(id: file.document.id, reason: reason, detail: detail)) != nil {
                        reporting = false
                        store.notice = t("examShare.reported")
                    }
                }
            }
        }
        .task { if file == nil { await open() } }
        .examShareFeedback(store)
    }

    private func open(download: Bool = false) async {
        failed = false
        if case let .document(value)? = await store.call(.open(id: route.id, download: download)) {
            file = value
            if download { store.notice = t("examShare.downloadedNotice") }
        } else {
            failed = file == nil
        }
    }

    private func save(_ saved: Bool) async {
        guard let current = file,
              case let .saved(updated)? = await store.call(.save(id: route.id, saved: saved)) else { return }
        var refreshed = current
        refreshed.saved = updated
        file = refreshed
    }
}

/// QuickLook inside the page rather than over it, so the document keeps the
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
/// it, see the shared one, or manage one's own.
struct ExamRequestScreen: View {
    @Environment(ExamShareStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    let route: ExamRequestRoute
    @State private var item: ExamRequest?
    @State private var uploading = false

    var body: some View {
        let item = item ?? route.request
        List {
            Section {
                VStack(alignment: .leading, spacing: 6) {
                    Text(item.subject.code).font(.footnote).foregroundStyle(Theme.muted)
                    Text(store.subjectName(item.subject)).font(.title3.weight(.semibold)).foregroundStyle(Theme.ink)
                    ExamStatusTag(status: item.status)
                }
                let facts = ExamRequestRow.facts(item)
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
                    Button(t("examShare.answerRequest"), systemImage: "square.and.arrow.up") { uploading = true }
                }
                if item.mine {
                    Button(t(item.status == "open" ? "examShare.closeRequest" : "examShare.reopenRequest")) {
                        Task { await setOpen(item.status != "open") }
                    }
                    Button(t("common.delete"), role: .destructive) {
                        store.confirm("examShare.deleteRequest") {
                            if await store.call(.deleteRequest(id: item.id)) != nil { dismiss() }
                        }
                    }
                }
                if let url = URL(string: ExamSite.url(.requests)) {
                    Link(destination: url) { Label(t("examShare.openInBrowser"), systemImage: "safari") }
                }
            }
        }
        .themedRows()
        .themedList()
        .navigationTitle(t("examShare.requests"))
        .navigationBarTitleDisplayMode(.inline)
        .sheet(isPresented: $uploading) {
            ExamUploadSheet(target: ExamUploadTarget(
                subjectCode: item.subject.code,
                subjectName: store.subjectName(item.subject),
                teachers: item.teachers,
                mine: item.teachers.map(\.id),
                academicYear: item.academicYear,
                occasion: item.occasion,
                role: item.wantedRole,
                requestId: item.id)) { dismiss() }
        }
        .examShareFeedback(store)
    }

    private func setOpen(_ open: Bool) async {
        guard await store.call(.setRequestOpen(id: (item ?? route.request).id, open: open)) != nil else { return }
        var changed = item ?? route.request
        changed.status = open ? "open" : "closed"
        item = changed
    }
}

import IsctCore
import SwiftUI

/// One request in full, then the one thing to do with it: share a file for
/// it, see the shared one, or close, reopen or delete one's own.
struct ExamRequestScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @Environment(\.push) private var push
    let route: ExamRequestRoute
    @State private var page = ExamPage()
    @State private var item: ExamRequest?
    @State private var uploadEntry: ExamUploadEntry?

    var body: some View {
        let item = item ?? route.request
        List {
            Section {
                VStack(alignment: .leading, spacing: Theme.space3) {
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
                    Button(t("examShare.upload.answerRequest"), systemImage: "square.and.arrow.up") {
                        Task { if await page.loadChoices(model) { uploadEntry = .request(item) } }
                    }
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
                if let web = page.site.flatMap({ URL(string: $0.requests) }) {
                    Link(destination: web) { Label(t("examShare.openOnWeb"), systemImage: "safari") }
                }
            }
        }
        .themedRows()
        .themedList()
        .disabled(page.busy)
        .navigationTitle(t("examShare.requests"))
        .navigationBarTitleDisplayMode(.inline)
        .task { await page.loadSite(model) }
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
        if let changed = board.items.first(where: { $0.id == item.id }) { self.item = changed }
    }
}

import IsctCore
import SwiftUI

/// ScienceTokyoExam, as the desktop shows it: this quarter's courses (or a
/// search), requests and the person's own documents, each a list that pushes
/// its course, request or document. Core decides everything shown here.
struct PastExamsScreen: View {
    @Environment(AppModel.self) private var model
    @State private var store = ExamShareStore()
    @State private var tab = Tab.browse

    enum Tab: String, CaseIterable { case browse, requests, library }

    var body: some View {
        VStack(spacing: 0) {
        PageHeader(title: t("examShare.title")) {
            if let url = URL(string: ExamSite.url(tab)) {
                Link(destination: url) {
                    Image(systemName: "safari").font(.title3)
                }
                .accessibilityLabel(t("examShare.openInBrowser"))
            }
        }
        Group {
            switch store.account?.status {
            case .connected:
                connected
            case nil:
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            default:
                ExamAccountGate()
            }
        }
        .frame(maxHeight: .infinity)
        }
        .background(Theme.bg)
        .toolbar(.hidden, for: .navigationBar)
        // Pushed pages do not inherit this page's environment: each gets the
        // store itself.
        .navigationDestination(for: ExamSubjectRoute.self) { ExamSubjectScreen(route: $0).environment(store) }
        .navigationDestination(for: ExamDocumentRoute.self) { ExamDocumentScreen(route: $0).environment(store) }
        .navigationDestination(for: ExamRequestRoute.self) { ExamRequestScreen(route: $0).environment(store) }
        .examShareFeedback(store)
        .environment(store)
        .task {
            store.model = model
            if store.account == nil { await store.account(.account) }
        }
    }

    private var connected: some View {
        VStack(spacing: 0) {
            Picker(t("examShare.title"), selection: $tab) {
                ForEach(Tab.allCases, id: \.self) { Text(t("examShare.\($0.rawValue)")).tag($0) }
            }
            .pickerStyle(.segmented)
            .padding(.horizontal, Theme.gutter)
            .padding(.vertical, 8)
            switch tab {
            case .browse: ExamBrowseList()
            case .requests: ExamRequestList()
            case .library: ExamLibraryList()
            }
        }
    }
}

/// The site's own page for what is on screen.
enum ExamSite {
    static let base = "https://sciencetokyoexam.com"
    static func url(_ tab: PastExamsScreen.Tab) -> String {
        switch tab {
        case .browse: "\(base)/search"
        case .requests: "\(base)/request"
        case .library: "\(base)/library"
        }
    }
    static func document(_ id: String) -> URL? {
        URL(string: "\(base)/view/\(id.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? id)")
    }
}

/// The account and the one Core call path every past-exams page shares.
@MainActor
@Observable
final class ExamShareStore {
    struct Confirmation {
        let key: String
        let work: () async -> Void
    }

    @ObservationIgnored weak var model: AppModel?
    var account: ExamAccount?
    var running = 0
    var problem: String?
    var notice: String?
    var confirmation: Confirmation?

    var busy: Bool { running > 0 }

    /// Runs one action. A failure shows its message, and a lost session
    /// resets the pages through the account snapshot.
    func call(_ action: ExamAction) async -> ExamAnswer? {
        guard let model else { return nil }
        running += 1
        problem = nil
        defer { running -= 1 }
        do { return try await model.examShare(action) }
        catch SdkError.Cancelled { return nil }
        catch is CancellationError { return nil }
        catch {
            problem = AppModel.describe(error)
            if case let .account(snapshot)? = try? await model.examShare(.accountSnapshot) {
                account = snapshot
            }
            return nil
        }
    }

    func account(_ action: ExamAction) async {
        if case let .account(value)? = await call(action) { account = value }
    }

    func confirm(_ key: String, work: @escaping () async -> Void) {
        confirmation = Confirmation(key: key, work: work)
    }

    func subjectName(_ subject: ExamSubject) -> String {
        Localizer.shared.pick(["ja": subject.name, "en": subject.nameEn ?? ""], content: model?.records.settings ?? [:])
            ?? subject.code
    }

    /// "2024 期末 2 · 試題", the desktop's sitting label.
    func sitting(year: UInt32?, occasion: String?, number: UInt32, variant: String) -> String {
        let occasion = occasion.map { value in
            let label = t("examShare.occasion.\(value)")
            return number > 1 ? "\(label) \(number)" : label
        }
        return [year.map(String.init) ?? t("examShare.unknownYearShort"), occasion, variant.isEmpty ? nil : variant]
            .compactMap { $0 }
            .joined(separator: " ")
    }

    func title(_ document: ExamDocument) -> String {
        [
            document.subjects.map(subjectName).joined(separator: " / "),
            sitting(year: document.academicYear, occasion: document.occasion,
                    number: document.occasionNumber, variant: document.variant),
            document.role.map { t("examShare.role.\($0)") },
        ]
        .compactMap { $0 }
        .filter { !$0.isEmpty }
        .joined(separator: " · ")
    }
}

extension View {
    /// The shared problem banner, notices and confirmations of a past-exams page.
    func examShareFeedback(_ store: ExamShareStore) -> some View {
        safeAreaInset(edge: .bottom) {
            if let message = store.problem ?? store.notice {
                Text(message)
                    .font(.subheadline)
                    .foregroundStyle(store.problem == nil ? Theme.ink : Theme.danger)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Theme.surface, in: RoundedRectangle(cornerRadius: Theme.radiusPanel, style: .continuous))
                    .padding(.horizontal, Theme.gutter)
                    .padding(.bottom, 8)
                    .onTapGesture { store.problem = nil; store.notice = nil }
                    .task(id: message) {
                        try? await Task.sleep(for: .seconds(4))
                        store.problem = nil
                        store.notice = nil
                    }
            }
        }
        .confirmationDialog(
            store.confirmation.map { t($0.key) } ?? "",
            isPresented: Binding(get: { store.confirmation != nil }, set: { if !$0 { store.confirmation = nil } }),
            titleVisibility: .visible,
            presenting: store.confirmation
        ) { confirmed in
            Button(t("common.ok"), role: .destructive) { Task { await confirmed.work() } }
        }
    }
}

/// Before a usable session: sign in, the terms, or why not.
private struct ExamAccountGate: View {
    @Environment(ExamShareStore.self) private var store

    var body: some View {
        ContentUnavailableView {
            Label(t("examShare.title"), systemImage: "doc.on.doc")
        } description: {
            switch store.account?.status {
            case .needsConsent: Text(t("examShare.consentHint"))
            case .unavailable: Text(t("error.exam-share-unsupported"))
            default: Text(store.busy ? t("examShare.connectingHint") : t("examShare.connectHint"))
            }
        } actions: {
            switch store.account?.status {
            case .needsConsent:
                Button(t("examShare.accept")) { Task { await store.account(.acceptTerms) } }
                    .buttonStyle(.borderedProminent)
                HStack(spacing: 16) {
                    if let terms = URL(string: "\(ExamSite.base)/terms") { Link(t("examShare.terms"), destination: terms) }
                    if let privacy = URL(string: "\(ExamSite.base)/privacy") { Link(t("examShare.privacy"), destination: privacy) }
                }
                .font(.subheadline)
            case .unavailable:
                EmptyView()
            default:
                Button(t("examShare.connect")) { Task { await store.account(.connect) } }
                    .buttonStyle(.borderedProminent)
                    .disabled(store.busy)
            }
            Text(t("examShare.partner")).font(.footnote).foregroundStyle(Theme.muted)
        }
    }
}

struct ExamSubjectRoute: Hashable {
    let code: String
    let title: String
    /// The timetable course's instructor text, for Core to match teachers.
    let instructor: String?
}

struct ExamDocumentRoute: Hashable {
    let id: String
}

struct ExamRequestRoute: Hashable {
    let request: ExamRequest
    static func == (a: Self, b: Self) -> Bool { a.request.id == b.request.id }
    func hash(into hasher: inout Hasher) { hasher.combine(request.id) }
}

/// This quarter's courses, or what a search finds; each pushes its course.
private struct ExamBrowseList: View {
    @Environment(AppModel.self) private var model
    @Environment(ExamShareStore.self) private var store
    @State private var courses: ExamCourseImportPreview?
    @State private var query = ""
    @State private var hits: (query: String, subjects: [ExamSubject])?

    var body: some View {
        let searching = !query.trimmingCharacters(in: .whitespaces).isEmpty
        List {
            Section {
                HStack(spacing: 8) {
                    Image(systemName: "magnifyingglass").foregroundStyle(Theme.muted)
                    TextField(t("examShare.search"), text: $query)
                        .submitLabel(.search)
                        .autocorrectionDisabled()
                    if !query.isEmpty {
                        Button { query = "" } label: {
                            Image(systemName: "xmark.circle.fill").foregroundStyle(Theme.muted)
                        }
                        .buttonStyle(.borderless)
                        .accessibilityLabel(t("common.clear"))
                    }
                }
            }
            if searching {
                if let hits, hits.query == query {
                    Section {
                        ForEach(hits.subjects, id: \.code) { subject in
                            NavigationLink(value: ExamSubjectRoute(
                                code: subject.code, title: store.subjectName(subject), instructor: nil)) {
                                row(store.subjectName(subject), [
                                    subject.code,
                                    subject.examCount.map { t("examShare.documentCount", ["count": $0]) },
                                ])
                            }
                        }
                    }
                    if hits.subjects.isEmpty { Text(t("examShare.noSubjects")).foregroundStyle(Theme.muted) }
                }
            } else if let courses {
                Section(t("examShare.myCourses")) {
                    ForEach(courses.courses, id: \.courseId) { course in
                        if let code = course.subjectCodes.first {
                            NavigationLink(value: ExamSubjectRoute(
                                code: code, title: course.title,
                                instructor: course.instructor.isEmpty ? nil : course.instructor)) {
                                row(course.title, [course.subjectCodes.joined(separator: " / "), course.instructor])
                            }
                        } else {
                            row(course.title, [t("examShare.noSubjectCode")]).foregroundStyle(Theme.muted)
                        }
                    }
                }
            }
            Section {} footer: { Text(t("examShare.partner")) }
        }
        .themedRows()
        .themedList()
        .pageListMargins()
        .task(id: query) {
            let trimmed = query.trimmingCharacters(in: .whitespaces)
            guard !trimmed.isEmpty else { return }
            try? await Task.sleep(for: .milliseconds(300))
            guard !Task.isCancelled else { return }
            if case let .subjects(found)? = await store.call(.search(query: trimmed)) {
                hits = (query, found)
            }
        }
        .task(id: model.contentRevision) {
            courses = await model.examCourseImport(term: nil)
        }
    }

    private func row(_ title: String, _ meta: [String?]) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(title).foregroundStyle(Theme.ink)
            let detail = meta.compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · ")
            if !detail.isEmpty { Text(detail).font(.footnote).foregroundStyle(Theme.muted) }
        }
    }
}

/// Requests from everyone, or the person's own; each pushes its detail.
private struct ExamRequestList: View {
    @Environment(ExamShareStore.self) private var store
    @State private var mine = false
    @State private var pages: [Bool: ExamRequestPage] = [:]

    var body: some View {
        let page = pages[mine]
        List {
            Section {
                Picker(t("examShare.requests"), selection: $mine) {
                    Text(t("examShare.openRequests")).tag(false)
                    Text(t("examShare.myRequests")).tag(true)
                }
                .pickerStyle(.segmented)
                .listRowBackground(Color.clear)
                .listRowInsets(EdgeInsets())
            }
            if let page {
                Section {
                    ForEach(page.items, id: \.id) { item in
                        NavigationLink(value: ExamRequestRoute(request: item)) { ExamRequestRow(item: item) }
                    }
                    if let cursor = page.cursor {
                        Button(t("examShare.more")) { Task { await load(cursor) } }
                    }
                }
                if page.items.isEmpty {
                    Text(t(mine ? "examShare.noMyRequests" : "examShare.noOpenRequests")).foregroundStyle(Theme.muted)
                }
            }
        }
        .themedRows()
        .themedList()
        .pageListMargins()
        .refreshable { await load() }
        .task(id: mine) { if pages[mine] == nil { await load() } }
        // Back from a request that changed: read the list again.
        .onAppear { if pages[mine] != nil { Task { await load() } } }
    }

    private func load(_ cursor: ExamRequestCursor? = nil) async {
        let mine = mine
        guard case let .requests(value)? = await store.call(.requests(mine: mine, cursor: cursor)) else { return }
        let previous = cursor == nil ? [] : pages[mine]?.items ?? []
        pages[mine] = ExamRequestPage(items: previous + value.items, cursor: value.cursor)
    }
}

struct ExamRequestRow: View {
    @Environment(ExamShareStore.self) private var store
    let item: ExamRequest

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 6) {
                Text(store.subjectName(item.subject)).foregroundStyle(Theme.ink)
                if item.status != "open" { ExamStatusTag(status: item.status) }
            }
            let detail = [ExamRequestRow.facts(item), item.description].filter { !$0.isEmpty }.joined(separator: " · ")
            if !detail.isEmpty {
                Text(detail).font(.footnote).foregroundStyle(Theme.muted).lineLimit(2)
            }
        }
    }

    static func facts(_ item: ExamRequest) -> String {
        [
            item.academicYear.map(String.init),
            item.occasion.map { t("examShare.occasion.\($0)") },
            item.wantedRole.map { t("examShare.role.\($0)") },
        ]
        .compactMap { $0 }
        .joined(separator: " · ")
    }
}

struct ExamStatusTag: View {
    let status: String
    var body: some View {
        let tint = status == "fulfilled" ? Theme.ok : status == "open" ? Theme.accent : Theme.muted
        Text(t("examShare.requestStatus.\(status)"))
            .font(.caption2.weight(.medium))
            .foregroundStyle(tint)
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .background(tint.opacity(0.12), in: Capsule())
    }
}

/// Saved, downloaded and shared documents; swiping removes a download or
/// withdraws a shared document.
private struct ExamLibraryList: View {
    @Environment(ExamShareStore.self) private var store
    @State private var library: ExamLibrary?

    var body: some View {
        List {
            if let library {
                if library.offline {
                    Text(t("examShare.libraryUnavailable")).foregroundStyle(Theme.muted)
                } else {
                    section("examShare.saved", library.saved)
                }
                section("examShare.downloaded", library.downloaded, removal: "examShare.removeDownload")
                if !library.offline {
                    section("examShare.submitted", library.submitted, removal: "examShare.deleteSubmission")
                }
            }
            // The account sits with the person's own things, out of the way.
            Section {
                if let email = store.account?.email {
                    LabeledContent(t("examShare.account"), value: email)
                }
                Button(t("examShare.disconnect"), role: .destructive) {
                    store.confirm("examShare.disconnectHint") { await store.account(.disconnect) }
                }
            } footer: {
                Text(t("examShare.partner"))
            }
        }
        .themedRows()
        .themedList()
        .pageListMargins()
        .refreshable { await load() }
        .task { await load() }
    }

    @ViewBuilder
    private func section(_ title: String, _ items: [ExamDocument], removal: String? = nil) -> some View {
        if !items.isEmpty {
            Section(t(title)) {
                ForEach(items, id: \.id) { item in
                    NavigationLink(value: ExamDocumentRoute(id: item.id)) {
                        VStack(alignment: .leading, spacing: 3) {
                            Text(item.subjects.map(store.subjectName).joined(separator: " / ")).foregroundStyle(Theme.ink)
                            Text([
                                store.sitting(year: item.academicYear, occasion: item.occasion,
                                              number: item.occasionNumber, variant: item.variant),
                                item.role.map { t("examShare.role.\($0)") },
                            ].compactMap { $0 }.joined(separator: " · "))
                            .font(.footnote).foregroundStyle(Theme.muted)
                        }
                    }
                    .swipeActions {
                        if let removal {
                            Button(t(removal), role: .destructive) {
                                store.confirm(removal) {
                                    let action: ExamAction = removal == "examShare.removeDownload"
                                        ? .removeDownload(id: item.id) : .deleteSubmission(id: item.id)
                                    if await store.call(action) != nil { await load() }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    private func load() async {
        if case let .library(value)? = await store.call(.library) { library = value }
    }
}

import IsctCore
import QuickLook
import SwiftUI

/// The course page, in the desktop course drawer's tabs: 概要 (room, links),
/// 資料 (the LMS sections and their files, opened in Quick Look) and 課題.
/// Ids and links come from Core's `CourseDetail`; rows are looked up in the
/// snapshot to render them.
struct CourseDetailScreen: View {
    let courseId: String
    @Environment(AppModel.self) private var model
    @State private var detail: CourseDetail?
    @State private var tab: Tab = .overview

    enum Tab: Hashable { case overview, materials, assignments }

    var body: some View {
        let course = model.records.courses[courseId] ?? [:]
        let tint = Color(hex: model.colors[courseId])
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                VStack(alignment: .leading, spacing: 4) {
                    Text(course.text("title") ?? "")
                        .font(.largeTitle.bold())
                        .foregroundStyle(Theme.ink)
                    Text(subtitle(course))
                        .font(.subheadline)
                        .foregroundStyle(Theme.muted)
                }
                Picker("表示", selection: $tab) {
                    Text("概要").tag(Tab.overview)
                    Text("資料").tag(Tab.materials)
                    Text("課題").tag(Tab.assignments)
                }
                .pickerStyle(.segmented)
                .sensoryFeedback(.selection, trigger: tab)
                switch tab {
                case .overview:
                    if let meeting = meetings(course).first {
                        roomCard(meeting, tint: tint)
                    }
                    links(title: course.text("title") ?? "")
                case .materials:
                    CourseMaterials(sections: sections, files: detail?.files, title: course.text("title") ?? "")
                case .assignments:
                    if assignments.isEmpty {
                        ContentUnavailableView("未提出の課題はありません", systemImage: "checkmark.circle")
                    } else {
                        Card {
                            ForEach(Array(assignments.enumerated()), id: \.element.assignmentId) { index, item in
                                if index > 0 { RowDivider() }
                                DeadlineLink(item: item) {
                                    DeadlineRow(item: item, now: Date(), showsCourse: false)
                                }
                            }
                        }
                    }
                }
            }
            .padding(.horizontal, 16)
            .padding(.bottom, 24)
        }
        .background(Theme.bg)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.visible, for: .navigationBar)
        .task(id: "\(courseId)-\(model.contentRevision)") {
            detail = await model.courseDetail(courseId)
        }
    }

    // MARK: Rows

    private struct Meeting {
        let day: Int
        let start: String
        let end: String
        let room: String
    }

    private func meetings(_ course: [String: Any]) -> [Meeting] {
        (course["meetings"] as? [[String: Any]] ?? []).compactMap { row in
            guard let start = row.text("startTime"), let end = row.text("endTime") else { return nil }
            let day = (row["days"] as? [NSNumber])?.first?.intValue ?? 0
            return Meeting(day: day, start: start, end: end, room: row.text("room") ?? "")
        }
    }

    private func subtitle(_ course: [String: Any]) -> String {
        let days = ["日", "月", "火", "水", "木", "金", "土"]
        var parts = [course.text("code")].compactMap { $0 }
        if let meeting = meetings(course).first {
            let start = model.periods.first { $0.startTime == meeting.start }?.periods.first
            let end = model.periods.first { $0.endTime == meeting.end }?.periods.last
            let periods = start.flatMap { s in end.flatMap { Format.periods([s, $0]) } }
            parts.append([days[min(max(meeting.day, 0), 6)], periods ?? "\(meeting.start)–\(meeting.end)"].joined(separator: " "))
        }
        if let credits = course.number("credits"), credits > 0 {
            parts.append("\(credits.formatted()) 単位")
        }
        return parts.joined(separator: " · ")
    }

    private func roomCard(_ meeting: Meeting, tint: Color) -> some View {
        HStack(alignment: .center) {
            VStack(alignment: .leading, spacing: 4) {
                Text(meeting.room.isEmpty ? "教室未定" : meeting.room)
                    .font(.title.bold())
                    .foregroundStyle(Theme.ink)
                Text("\(meeting.start) – \(meeting.end)")
                    .font(.subheadline.monospacedDigit())
                    .foregroundStyle(tint)
            }
            Spacer()
        }
        .padding(18)
        .background(tint.opacity(Theme.tintSoft), in: RoundedRectangle(cornerRadius: Theme.radiusPanel, style: .continuous))
    }

    /// This course's open work from Core's task timeline, in its order.
    private var assignments: [DeadlineItem] {
        guard let today = model.board?.today else { return [] }
        return (today.pinned + today.dueThisWeek + today.later + today.pastRecent)
            .filter { $0.courseId == courseId }
    }

    /// Every material in Core's section order, grouped by section as LMS
    /// shows them; the newest section comes first.
    private var sections: [MaterialSection] {
        var sections: [MaterialSection] = []
        for id in detail?.materialIds ?? [] {
            guard let row = model.records.materials[id] else { continue }
            let number = (row["sectionNumber"] as? NSNumber ?? row["session"] as? NSNumber)?.intValue
            let title = row.text("sectionTitle") ?? number.map { "第\($0)回" } ?? "その他"
            let item = MaterialSection.Item(
                id: id, title: row.text("title") ?? "", kind: row.text("type") ?? "link",
                source: row.text("url") ?? row.text("resourceUrl"))
            if sections.last?.title == title {
                sections[sections.count - 1].items.append(item)
            } else {
                sections.append(MaterialSection(title: title, items: [item]))
            }
        }
        return sections.reversed()
    }

    @ViewBuilder
    private func links(title: String) -> some View {
        let lms = detail?.lmsUrl
        let syllabus = detail?.syllabusUrl
        if lms != nil || syllabus != nil {
            Card {
                if let lms {
                    NavigationLink(value: WebRouter.PushedPage(site: .scienceLms, target: lms, title: title)) {
                        linkRow("T2SCHOLA で開く", icon: "globe")
                    }
                    .buttonStyle(.plain)
                }
                if lms != nil, syllabus != nil { RowDivider() }
                if let syllabus {
                    NavigationLink(value: WebRouter.PushedPage(site: .syllabus, target: syllabus, title: title)) {
                        linkRow("シラバス", icon: "doc.text")
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.top, 8)
        }
    }

    private func linkRow(_ title: LocalizedStringKey, icon: String) -> some View {
        HStack(spacing: 12) {
            Image(systemName: icon).foregroundStyle(Theme.muted).frame(width: 16)
            Text(title).foregroundStyle(Theme.ink)
            Spacer()
            Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(Theme.muted)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 14)
        .contentShape(Rectangle())
    }
}

struct MaterialSection: Identifiable {
    struct Item: Identifiable {
        let id: String
        let title: String
        let kind: String
        /// The LMS page or file URL Core resolves and downloads.
        let source: String?
    }

    let title: String
    var items: [Item]
    var id: String { title }
}

/// 資料: sections and files. Tapping a file asks Core for it (downloaded with
/// the signed-in LMS session, or from the course folder) and shows it in
/// Quick Look; what Core cannot fetch opens in the in-app LMS page.
private struct CourseMaterials: View {
    let sections: [MaterialSection]
    let files: CourseKey?
    let title: String
    @Environment(AppModel.self) private var model
    @Environment(Navigator.self) private var navigator
    @State private var opening: String?
    @State private var preview: URL?
    @State private var notice: String?
    @State private var opened = 0

    var body: some View {
        if sections.isEmpty {
            ContentUnavailableView("資料はまだありません", systemImage: "doc.text")
        } else {
            VStack(alignment: .leading, spacing: 12) {
                ForEach(sections) { section in
                    SectionLabel(LocalizedStringKey(section.title))
                    Card {
                        ForEach(Array(section.items.enumerated()), id: \.element.id) { index, item in
                            if index > 0 { RowDivider() }
                            row(item)
                        }
                    }
                }
            }
            .quickLookPreview($preview)
            .sensoryFeedback(.success, trigger: opened)
            .alert(notice ?? "", isPresented: Binding(get: { notice != nil }, set: { if !$0 { notice = nil } })) {
                Button("OK", role: .cancel) {}
            }
        }
    }

    private func row(_ item: MaterialSection.Item) -> some View {
        Button {
            Task { await open(item) }
        } label: {
            HStack(spacing: 12) {
                Image(systemName: icon(item.kind)).foregroundStyle(Theme.muted).frame(width: 18)
                Text(item.title).foregroundStyle(Theme.ink).lineLimit(2).multilineTextAlignment(.leading)
                Spacer(minLength: 8)
                if opening == item.id {
                    ProgressView().controlSize(.small)
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(opening != nil)
    }

    private func open(_ item: MaterialSection.Item) async {
        guard let source = item.source else { return }
        guard let files else {
            push(source, item.title)
            return
        }
        opening = item.id
        defer { opening = nil }
        switch await model.openMaterial(files, source: source) {
        case let .ready(file)?:
            preview = URL(fileURLWithPath: file.path)
            opened += 1
        case let .tooLarge(bytes)?:
            notice = "このファイルは大きすぎます（\(ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .file))）。LMS で開いてください。"
        case .failed(.login)?:
            notice = "LMS に再ログインが必要です。設定から学校アカウントを確認してください。"
        case let .external(url)?:
            push(url, item.title)
        default:
            // Folders, pages and anything Core could not fetch: the LMS page itself.
            push(source, item.title)
        }
    }

    private func push(_ url: String, _ title: String) {
        navigator.push(WebRouter.PushedPage(site: .scienceLms, target: url, title: title))
    }

    private func icon(_ kind: String) -> String {
        switch kind {
        case "pdf": "doc.richtext"
        case "video": "play.rectangle"
        case "folder": "folder"
        case "page": "doc.text"
        default: "link"
        }
    }
}

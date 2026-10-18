import IsctCore
import SwiftUI

/// Course detail: room card with map, open assignments, latest materials and
/// the course's T2SCHOLA and syllabus pages. Ids and links come from Core's
/// `CourseDetail`; rows are looked up in the snapshot to render them.
struct CourseDetailScreen: View {
    let courseId: String
    @Environment(AppModel.self) private var model
    @State private var detail: CourseDetail?

    private static let latestMaterials = 3

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
                if let meeting = meetings(course).first {
                    roomCard(meeting, tint: tint)
                }
                if !assignments.isEmpty {
                    SectionLabel("課題")
                    Card {
                        ForEach(Array(assignments.enumerated()), id: \.element.assignmentId) { index, item in
                            if index > 0 { RowDivider() }
                            DeadlineLink(item: item) {
                                DeadlineRow(item: item, now: Date(), showsCourse: false)
                            }
                        }
                    }
                }
                if !materials.isEmpty {
                    SectionLabel("資料")
                    Card {
                        ForEach(Array(materials.enumerated()), id: \.element.materialId) { index, item in
                            if index > 0 { RowDivider() }
                            MaterialRow(item: item)
                        }
                    }
                }
                links(title: course.text("title") ?? "")
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
            if let query = roomMapQuery(room: meeting.room) {
                MapButton(query: query, title: "地図", tint: tint)
            }
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

    /// The newest materials: Core orders by section, so the last ids are newest.
    private var materials: [MaterialItem] {
        (detail?.materialIds ?? []).suffix(Self.latestMaterials).reversed().compactMap { id in
            guard let row = model.records.materials[id] else { return nil }
            return MaterialItem(
                materialId: id, courseId: courseId, courseTitle: nil, color: nil,
                title: row.text("title") ?? "", kind: row.text("type") ?? "link",
                section: (row["sectionNumber"] as? NSNumber ?? row["session"] as? NSNumber)?.int64Value,
                sectionTitle: row.text("sectionTitle"),
                url: row.text("resourceUrl") ?? row.text("url"),
                updatedAt: row.text("updatedAt"), addedToday: false)
        }
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

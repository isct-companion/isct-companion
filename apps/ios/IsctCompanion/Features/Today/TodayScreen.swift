import IsctCore
import SwiftUI

/// 今日: one hero chosen by Core for this moment, then at most two short
/// sections. The shell renders `TodayView` in order and never sorts or filters.
struct TodayScreen: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        TimelineView(.periodic(from: .now, by: 30)) { context in
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    header
                    if let problem = model.problem {
                        ProblemBanner(text: problem)
                    } else if case let .problem(site, problem)? = model.schoolStatus?.headline {
                        SchoolProblemBanner(site: site, problem: problem)
                    }
                    if model.needsAccount { ConnectCard() }
                    if let status = model.schoolStatus, status.headline.ringProgress(status) != nil,
                        model.records.courses.isEmpty, model.records.assignments.isEmpty
                    {
                        FirstLoadCard(status: status)
                    }
                    if let today = model.today {
                        TodayContent(today: today, now: context.date)
                    }
                }
                .padding(.horizontal, 16)
                .padding(.bottom, 24)
            }
            .refreshable { model.refreshFromSchool() }
        }
        .background(Theme.bg)
        .toolbar(.hidden, for: .navigationBar)
        .safeAreaInset(edge: .bottom) { UndoToast() }
    }

    private var header: some View {
        HStack(alignment: .bottom) {
            VStack(alignment: .leading, spacing: 2) {
                Text(subtitle)
                    .font(.subheadline)
                    .foregroundStyle(Theme.muted)
                Text("今日")
                    .font(.largeTitle.bold())
                    .foregroundStyle(Theme.ink)
            }
            Spacer()
            SyncRingButton()
            NavigationLink(value: SettingsRoute.root) {
                Image(systemName: "gearshape")
                    .font(.title3)
                    .foregroundStyle(Theme.ink)
                    .frame(width: 40, height: 40)
                    .background(Theme.surface2, in: Circle())
            }
            .accessibilityLabel("設定")
        }
        .padding(.top, 12)
    }

    private var subtitle: String {
        guard let today = model.today else { return "" }
        var parts = [Format.dayTitle(today.date)]
        if today.noClasses {
            parts.append(today.calendarNotice ?? "休日")
        } else if let week = today.teachingWeek {
            parts.append("\(Format.quarter(today.term)) 第\(week)週")
        }
        return parts.joined(separator: " · ")
    }
}

private struct TodayContent: View {
    let today: TodayView
    let now: Date

    var body: some View {
        switch today.phase {
        case .beforeClasses, .betweenClasses:
            if let hero = today.hero { NextClassHero(item: hero, now: now) }
            if !today.later.isEmpty {
                SectionLabel("このあと")
                ClassList(items: today.later)
            }
            DeadlineSection(title: "期限", items: today.deadlines, now: now)
        case .inClass:
            if let hero = today.hero { CurrentClassHero(item: hero, now: now) }
            if !today.classMaterials.isEmpty || !today.classAssignments.isEmpty {
                SectionLabel("この授業")
                ThisClassCard(materials: today.classMaterials, assignments: today.classAssignments, now: now)
            }
            if let next = today.later.first, let hero = today.hero {
                SectionLabel("次")
                ClassList(items: [next], breakAfter: hero)
            }
            DeadlineSection(title: "期限", items: today.deadlines, now: now)
        case .afterClasses:
            if let hero = today.deadlineHero { TonightHero(item: hero, now: now) }
            if let next = today.nextClass {
                SectionLabel(Format.civilDate(next.occurrence.date).map { Calendar.tokyo.isDateInTomorrow($0) } == true ? "明日の授業" : "次の授業")
                NextDayCard(item: next)
            }
            DeadlineSection(title: "期限", items: today.deadlines, now: now)
        case .noClasses:
            DeadlineSection(title: "期限", items: today.deadlines, now: now)
            if !today.recent.isEmpty {
                SectionLabel("新着")
                RecentCard(items: today.recent)
            }
            if let next = today.nextClass {
                SectionLabel("次の授業")
                NextDayCard(item: next)
            }
        }
    }
}

// MARK: Heroes

/// Before or between classes: start time and room in large figures.
private struct NextClassHero: View {
    let item: TodayClass
    let now: Date

    var body: some View {
        let tint = Color(hex: item.occurrence.color)
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text(["次の授業", Format.periods(item.periods)].compactMap { $0 }.joined(separator: " · "))
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(tint)
                Spacer()
                if let starts = Format.date(item.startsAt) {
                    let minutes = max(Format.minutes(until: starts, now: now), 0)
                    TagChip(text: "あと \(minutes) 分", ink: tint, fill: Theme.surface)
                        .countingFigure(minutes)
                }
            }
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                Text(item.occurrence.startTime)
                    .font(.system(size: 48, weight: .bold).monospacedDigit())
                Text(item.occurrence.room)
                    .font(.title.bold())
            }
            .foregroundStyle(Theme.ink)
            .minimumScaleFactor(0.6)
            .lineLimit(1)
            CourseLink(id: item.occurrence.courseId, title: item.occurrence.title, zoom: "hero")
        }
        .heroCard(tint: tint)
        .courseZoomSource("hero")
    }
}

/// In class: minutes left, progress and end time.
private struct CurrentClassHero: View {
    let item: TodayClass
    let now: Date

    var body: some View {
        let tint = Color(hex: item.occurrence.color)
        let starts = Format.date(item.startsAt) ?? now
        let ends = Format.date(item.endsAt) ?? now
        let total = max(ends.timeIntervalSince(starts), 1)
        let progress = min(max(now.timeIntervalSince(starts) / total, 0), 1)
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text((["授業中", Format.periods(item.periods), item.occurrence.room] as [String?]).compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · "))
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(tint)
                Spacer()
                Text("\(item.occurrence.endTime) まで")
                    .font(.subheadline.weight(.semibold).monospacedDigit())
                    .foregroundStyle(tint)
            }
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Text("あと").font(.title3.weight(.semibold))
                let minutes = max(Format.minutes(until: ends, now: now), 0)
                Text("\(minutes)")
                    .font(.system(size: 48, weight: .bold).monospacedDigit())
                    .countingFigure(minutes)
                Text("分").font(.title3.weight(.semibold))
            }
            .foregroundStyle(Theme.ink)
            ProgressView(value: progress)
                .tint(tint)
                .animation(.smooth(duration: 0.8), value: progress)
            CourseLink(id: item.occurrence.courseId, title: item.occurrence.title, zoom: "hero")
        }
        .heroCard(tint: tint)
        .courseZoomSource("hero")
    }
}

/// After classes: tonight's nearest deadline on the danger tint.
private struct TonightHero: View {
    let item: DeadlineItem
    let now: Date
    @Environment(WebRouter.self) private var web

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text("今夜の期限")
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(Theme.danger)
                Spacer()
                DeadlineChip(item: item, now: now)
            }
            Text(Format.due(item.dueAt, now: now).replacingOccurrences(of: "今日 ", with: ""))
                .font(.system(size: 48, weight: .bold).monospacedDigit())
                .foregroundStyle(Theme.ink)
            Text(item.title).font(.title3.weight(.semibold)).foregroundStyle(Theme.ink)
            if let course = item.courseTitle {
                Text(course).font(.subheadline).foregroundStyle(Theme.muted)
            }
            if let url = item.sourceUrl {
                NavigationLink(value: WebRouter.PushedPage(site: .scienceLms, target: url, title: item.title)) {
                    Label("提出ページを開く", systemImage: "arrow.up.forward.square")
                        .font(.body.weight(.semibold))
                        .foregroundStyle(.white)
                        .padding(.horizontal, 16)
                        .padding(.vertical, 10)
                        .background(Theme.danger, in: Capsule())
                }
            }
        }
        .padding(18)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Theme.dangerSoft, in: RoundedRectangle(cornerRadius: Theme.radiusPanel, style: .continuous))
    }
}

// MARK: Sections

private struct CourseLink: View {
    let id: String?
    let title: String
    var zoom: String? = nil
    var body: some View {
        if let id {
            NavigationLink(value: CourseRoute(id: id, zoom: zoom)) { label }
        } else {
            label
        }
    }
    private var label: some View {
        HStack(spacing: 4) {
            Text(title).font(.title3.weight(.semibold)).lineLimit(2).multilineTextAlignment(.leading)
            if id != nil { Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(Theme.muted) }
        }
        .foregroundStyle(Theme.ink)
    }
}

/// Rows of classes: time, course square, title and periods, room.
private struct ClassList: View {
    let items: [TodayClass]
    /// When set, the first row shows the break after this class.
    var breakAfter: TodayClass?

    var body: some View {
        Card {
            ForEach(Array(items.enumerated()), id: \.offset) { index, item in
                if index > 0 { RowDivider(inset: 76) }
                row(item)
            }
        }
    }

    private func row(_ item: TodayClass) -> some View {
        let content = HStack(spacing: 12) {
            Text(item.occurrence.startTime)
                .font(.headline.monospacedDigit())
                .foregroundStyle(Theme.ink)
                .frame(width: 48, alignment: .leading)
            CourseSquare(color: item.occurrence.color)
            VStack(alignment: .leading, spacing: 2) {
                Text(item.occurrence.title).font(.body).foregroundStyle(Theme.ink).lineLimit(1)
                if let detail = detail(item) {
                    Text(detail).font(.subheadline).foregroundStyle(Theme.muted)
                }
            }
            Spacer(minLength: 8)
            Text(item.occurrence.room)
                .font(.headline)
                .foregroundStyle(Theme.ink)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .contentShape(Rectangle())
        return Group {
            if let id = item.occurrence.courseId {
                NavigationLink(value: CourseRoute(id: id)) { content }.buttonStyle(.plain)
            } else {
                content
            }
        }
    }

    private func detail(_ item: TodayClass) -> String? {
        var parts = [Format.periods(item.periods)].compactMap { $0 }
        if let previous = breakAfter, let ends = Format.date(previous.endsAt), let starts = Format.date(item.startsAt) {
            parts.append("休憩 \(Int(starts.timeIntervalSince(ends) / 60)) 分")
        }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }
}

private struct DeadlineSection: View {
    let title: LocalizedStringKey
    let items: [DeadlineItem]
    let now: Date
    @Environment(AppModel.self) private var model

    var body: some View {
        if !items.isEmpty {
            SectionLabel(title)
            Card {
                ForEach(Array(items.enumerated()), id: \.element.assignmentId) { index, item in
                    if index > 0 { RowDivider() }
                    DeadlineLink(item: item) { DeadlineRow(item: item, now: now) }
                }
            }
        }
    }
}

/// Opens the assignment's submission page; long press offers quick actions.
struct DeadlineLink<Content: View>: View {
    let item: DeadlineItem
    @ViewBuilder var label: Content
    @Environment(AppModel.self) private var model
    @Environment(Navigator.self) private var navigator

    var body: some View {
        NavigationLink(value: WebRouter.PushedPage(site: .scienceLms, target: item.sourceUrl, title: item.title)) {
            label
        }
        .buttonStyle(.plain)
        .contextMenu {
            // A NavigationLink inside a menu does not push reliably.
            if let course = item.courseId {
                Button("授業を開く", systemImage: "book") { navigator.push(CourseRoute(id: course)) }
            }
            Button {
                Task { await model.setPinned(item, !item.pinned) }
            } label: {
                Label(item.pinned ? "ピン留めを外す" : "ピン留め", systemImage: "pin")
            }
            Button(role: .destructive) {
                Task { await model.setHidden(item, true) }
            } label: {
                Label("非表示", systemImage: "eye.slash")
            }
        }
    }
}

/// In class: the newest material and open work of this course.
private struct ThisClassCard: View {
    let materials: [MaterialItem]
    let assignments: [DeadlineItem]
    let now: Date

    var body: some View {
        Card {
            ForEach(Array(materials.enumerated()), id: \.element.materialId) { index, item in
                if index > 0 { RowDivider() }
                MaterialRow(item: item)
            }
            ForEach(Array(assignments.enumerated()), id: \.element.assignmentId) { index, item in
                if index > 0 || !materials.isEmpty { RowDivider() }
                DeadlineLink(item: item) {
                    HStack(spacing: 12) {
                        Image(systemName: "doc.text").foregroundStyle(Theme.muted).frame(width: 16)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(item.title).foregroundStyle(Theme.ink).lineLimit(2)
                            Text(Format.due(item.dueAt, now: now)).font(.subheadline).foregroundStyle(Theme.muted)
                        }
                        Spacer(minLength: 8)
                        DeadlineChip(item: item, now: now)
                    }
                    .padding(.horizontal, 16)
                    .padding(.vertical, 10)
                }
            }
        }
    }
}

struct MaterialRow: View {
    let item: MaterialItem
    @Environment(\.openURL) private var openURL

    var body: some View {
        Button {
            if let raw = item.url, let url = URL(string: raw) { openURL(url) }
        } label: {
            HStack(spacing: 12) {
                Image(systemName: icon).foregroundStyle(Theme.muted).frame(width: 16)
                VStack(alignment: .leading, spacing: 2) {
                    Text(item.title).foregroundStyle(Theme.ink).lineLimit(2)
                    if let detail {
                        Text(detail).font(.subheadline).foregroundStyle(Theme.muted).lineLimit(1)
                    }
                }
                Spacer(minLength: 8)
                if item.addedToday { TagChip(text: "今日追加") }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    private var icon: String {
        switch item.kind {
        case "pdf": return "doc"
        case "video": return "play.rectangle"
        default: return "link"
        }
    }

    private var detail: String? {
        // LMS section titles usually carry their own number; fall back to it.
        item.sectionTitle ?? item.section.map { "第\($0)回" }
    }
}

/// After classes or on a day off: the next class day's first class.
private struct NextDayCard: View {
    let item: TodayClass

    var body: some View {
        let date = Format.civilDate(item.occurrence.date)
        let parts = date.map { Calendar.tokyo.dateComponents([.weekday, .day], from: $0) }
        let content = HStack(spacing: 14) {
            VStack(spacing: 0) {
                Text(parts.map { ["日", "月", "火", "水", "木", "金", "土"][($0.weekday ?? 1) - 1] } ?? "")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(Theme.muted)
                Text(parts?.day.map(String.init) ?? "")
                    .font(.title2.weight(.bold).monospacedDigit())
                    .foregroundStyle(Theme.ink)
            }
            .frame(width: 52, height: 56)
            .background(Theme.surface2, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 8) {
                    Text(item.occurrence.startTime).font(.title3.weight(.bold).monospacedDigit())
                    Text(item.occurrence.room).font(.headline).lineLimit(1).minimumScaleFactor(0.8)
                }
                .foregroundStyle(Theme.ink)
                Text(item.occurrence.title).font(.body).foregroundStyle(Theme.ink).lineLimit(1)
            }
            Spacer()
            Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(Theme.muted)
        }
        .padding(14)
        Card {
            if let id = item.occurrence.courseId {
                NavigationLink(value: CourseRoute(id: id)) { content }.buttonStyle(.plain)
            } else {
                content
            }
        }
    }
}

/// Day off: grades and materials that changed recently.
private struct RecentCard: View {
    let items: [RecentItem]

    var body: some View {
        Card {
            ForEach(Array(items.enumerated()), id: \.offset) { index, item in
                if index > 0 { RowDivider() }
                switch item {
                case let .grade(row):
                    recentRow(color: row.color, title: [row.title, row.grade].compactMap { $0 }.joined(separator: " · "),
                              detail: row.courseTitle, tag: TagChip(text: "採点済", ink: Theme.ok, fill: Theme.okSoft), courseId: row.courseId)
                case let .material(row):
                    recentRow(color: row.color, title: row.title, detail: row.courseTitle,
                              tag: TagChip(text: "資料"), courseId: row.courseId)
                }
            }
        }
    }

    private func recentRow(color: String?, title: String, detail: String?, tag: TagChip, courseId: String?) -> some View {
        let content = HStack(spacing: 12) {
            CourseSquare(color: color)
            VStack(alignment: .leading, spacing: 2) {
                Text(title).foregroundStyle(Theme.ink).lineLimit(2)
                if let detail { Text(detail).font(.subheadline).foregroundStyle(Theme.muted).lineLimit(1) }
            }
            Spacer(minLength: 8)
            tag
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .contentShape(Rectangle())
        return Group {
            if let courseId {
                NavigationLink(value: CourseRoute(id: courseId)) { content }.buttonStyle(.plain)
            } else {
                content
            }
        }
    }
}

// MARK: Feedback

struct ProblemBanner: View {
    let text: String
    @Environment(AppModel.self) private var model

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: "exclamationmark.circle.fill").foregroundStyle(Theme.danger)
            Text(text).font(.subheadline).foregroundStyle(Theme.ink)
            Spacer()
            Button {
                model.dismissProblem()
            } label: {
                Image(systemName: "xmark").font(.footnote.weight(.semibold)).foregroundStyle(Theme.muted)
            }
        }
        .padding(14)
        .background(Theme.dangerSoft, in: RoundedRectangle(cornerRadius: Theme.radiusSurface, style: .continuous))
    }
}

/// First launch: the real empty Today with one sign-in card.
private struct ConnectCard: View {

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("学校アカウントを接続").font(.title3.weight(.bold)).foregroundStyle(Theme.ink)
            Text("時間割・課題・資料を学校のサイトから読み込みます。パスワードはキーチェーンに保存され、学校のサイトにだけ送られます。")
                .font(.subheadline)
                .foregroundStyle(Theme.muted)
            NavigationLink(value: SettingsRoute.account(.science)) {
                Text("Science Tokyo で接続")
                    .font(.body.weight(.semibold))
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 12)
                    .foregroundStyle(.white)
                    .background(Theme.accent, in: Capsule())
            }
        }
        .padding(18)
        .background(Theme.surface, in: RoundedRectangle(cornerRadius: Theme.radiusPanel, style: .continuous))
    }
}

struct UndoToast: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if let undo = model.undo {
            HStack {
                Text("「\(undo.title)」を非表示にしました").lineLimit(1)
                Spacer()
                Button("取り消す") { Task { await model.undoHide() } }.fontWeight(.semibold)
            }
            .font(.subheadline)
            .foregroundStyle(.white)
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            .background(Color.black.opacity(0.85), in: Capsule())
            .padding(.horizontal, 16)
            .padding(.bottom, 8)
            .transition(.move(edge: .bottom).combined(with: .opacity))
            .task(id: undo) {
                try? await Task.sleep(for: .seconds(5))
                model.dismissUndo()
            }
        }
    }
}

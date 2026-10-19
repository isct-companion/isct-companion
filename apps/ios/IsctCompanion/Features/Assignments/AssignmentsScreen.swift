import IsctCore
import SwiftUI

/// 課題: what is left to do this quarter, in the groups of Core's task
/// timeline (`AssignmentBoard.today`). Swipe right pins, swipe left hides;
/// every assignment, submitted and hidden included, sits behind one row.
struct AssignmentsScreen: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        TimelineView(.periodic(from: .now, by: 60)) { context in
            List {
                Section {
                    VStack(alignment: .leading, spacing: 2) {
                        HStack(spacing: 6) {
                            Text("\(Format.quarter(model.board?.term ?? "")) · 未提出のみ")
                                .font(.subheadline)
                                .foregroundStyle(Theme.muted)
                            if let status = model.schoolStatus, let progress = status.headline.ringProgress(status) {
                                SyncRing(progress: progress, emphasized: status.headline.isBusy).scaleEffect(0.8)
                            }
                        }
                        Text("課題").font(.largeTitle.bold()).foregroundStyle(Theme.ink)
                    }
                    .listRowBackground(Color.clear)
                    .listRowInsets(EdgeInsets(top: 12, leading: 4, bottom: 0, trailing: 4))
                }
                if let problem = model.problem {
                    Section { ProblemBanner(text: problem) }
                        .listRowBackground(Color.clear)
                        .listRowInsets(EdgeInsets())
                }
                if let today = model.board?.today {
                    group("ピン留め", today.pinned, now: context.date)
                    group("今週", today.dueThisWeek, now: context.date)
                    group("来週以降", today.later, now: context.date)
                    group("期限切れ", today.pastRecent, now: context.date)
                    if today.isOpenEmpty {
                        Section {
                            ContentUnavailableView("未提出の課題はありません", systemImage: "checkmark.circle")
                        }
                        .listRowBackground(Color.clear)
                    }
                    Section {
                        NavigationLink(value: AssignmentRecordRoute()) {
                            Text("提出済・非表示を含むすべて").foregroundStyle(Theme.ink)
                        }
                    }
                }
            }
            .listStyle(.insetGrouped)
            .scrollContentBackground(.hidden)
            .background(Theme.bg)
            .refreshable { model.refreshFromSchool() }
        }
        .toolbar(.hidden, for: .navigationBar)
        .safeAreaInset(edge: .bottom) { UndoToast() }
    }

    @ViewBuilder
    private func group(_ title: LocalizedStringKey, _ items: [DeadlineItem], now: Date) -> some View {
        if !items.isEmpty {
            Section(title) {
                ForEach(items, id: \.assignmentId) { item in
                    AssignmentListRow(item: item) {
                        DeadlineRow(item: item, now: now, overdueStyle: .elapsed)
                    }
                }
            }
        }
    }
}

extension DeadlineGroups {
    /// Nothing left to do among the groups the 課題 tab shows.
    var isOpenEmpty: Bool {
        pinned.isEmpty && dueThisWeek.isEmpty && later.isEmpty && undated.isEmpty && pastRecent.isEmpty
    }
}

/// Every assignment of the quarter whatever its state (`AssignmentBoard.record`);
/// older past work stays folded, hidden work comes back from here.
struct RecordScreen: View {
    @Environment(AppModel.self) private var model
    @State private var unfolded = false

    var body: some View {
        List {
            if let record = model.board?.record {
                group("ピン留め", record.pinned)
                group("今週", record.dueThisWeek)
                group("来週以降", record.later)
                group("期限なし", record.undated)
                group("過去", record.pastRecent)
                if !record.pastEarlier.isEmpty {
                    if unfolded {
                        group("それ以前", record.pastEarlier)
                    } else {
                        Section {
                            Button("それ以前の \(record.pastEarlier.count) 件を表示") { unfolded = true }
                        }
                    }
                }
                if !record.hidden.isEmpty {
                    Section("非表示") {
                        ForEach(record.hidden, id: \.assignmentId) { item in
                            AssignmentListRow(item: item, hidden: true) { DeadlineRow(item: item, now: Date()) }
                        }
                    }
                }
            }
        }
        .listStyle(.insetGrouped)
        .scrollContentBackground(.hidden)
        .background(Theme.bg)
        .navigationTitle("すべての課題")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.visible, for: .navigationBar)
    }

    @ViewBuilder
    private func group(_ title: LocalizedStringKey, _ items: [DeadlineItem]) -> some View {
        if !items.isEmpty {
            Section(title) {
                ForEach(items, id: \.assignmentId) { item in
                    AssignmentListRow(item: item) { DeadlineRow(item: item, now: Date()) }
                }
            }
        }
    }
}

/// One assignment in a 課題 list. A plain button rather than a NavigationLink,
/// so List draws no disclosure chevron, and with the system row highlight
/// rather than `PressableStyle`, whose dip on touch-down fights the swipe.
/// Swipe right pins (full swipe allowed); swipe left hides, never by a long
/// pull alone. A hidden row offers only 戻す.
private struct AssignmentListRow<Content: View>: View {
    let item: DeadlineItem
    var hidden = false
    @ViewBuilder var label: Content
    @Environment(AppModel.self) private var model
    @Environment(Navigator.self) private var navigator
    /// Bumped by every pin or hide, so each one answers with a tap.
    @State private var acted = 0

    var body: some View {
        Button {
            navigator.push(AssignmentRoute(id: item.assignmentId))
        } label: {
            label
        }
        .listRowInsets(EdgeInsets())
        .contextMenu {
            if let course = item.courseId {
                Button("授業を開く", systemImage: "book") { navigator.push(CourseRoute(id: course)) }
            }
            if let url = item.sourceUrl {
                Button("LMS で提出", systemImage: "arrow.up.forward.square") {
                    navigator.push(WebRouter.PushedPage(site: .scienceLms, target: url, title: item.title))
                }
            }
            if hidden {
                Button("戻す", systemImage: "eye") { setHidden(false) }
            } else {
                Button(item.pinned ? "ピン留めを外す" : "ピン留め", systemImage: "pin") { togglePin() }
                Button("非表示", systemImage: "eye.slash", role: .destructive) { setHidden(true) }
            }
        }
        .swipeActions(edge: .leading) {
            if !hidden {
                Button {
                    togglePin()
                } label: {
                    Label(item.pinned ? "外す" : "ピン留め", systemImage: item.pinned ? "pin.slash" : "pin")
                }
                .tint(Theme.accent)
            }
        }
        .swipeActions(edge: .trailing, allowsFullSwipe: hidden) {
            if hidden {
                Button { setHidden(false) } label: { Label("戻す", systemImage: "eye") }
                    .tint(Theme.accent)
            } else {
                Button { setHidden(true) } label: { Label("非表示", systemImage: "eye.slash") }
                    .tint(Theme.muted)
            }
        }
        .sensoryFeedback(.impact(weight: .light), trigger: acted)
    }

    private func togglePin() {
        acted += 1
        Task { await model.setPinned(item, !item.pinned) }
    }

    private func setHidden(_ hide: Bool) {
        acted += 1
        Task { await model.setHidden(item, hide) }
    }
}

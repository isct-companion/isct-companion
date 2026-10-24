import IsctCore
import SwiftUI
import TipKit

/// 今日's 課題: what is left to do this quarter, in the groups of Core's task
/// timeline (`AssignmentBoard.today`), as sections of 今日's list so the rows
/// keep the system swipes: right pins, left hides. Every assignment,
/// submitted and hidden included, sits behind すべて.
struct TodayAssignmentSections: View {
    let now: Date
    @Environment(AppModel.self) private var model
    @Environment(\.push) private var push

    var body: some View {
        if let board = model.board {
            Section {
                if board.today.isOpenEmpty {
                    Text(t("No unsubmitted assignments"))
                        .foregroundStyle(Theme.muted)
                        .listRowBackground(Color.clear)
                } else {
                    TipView(AssignmentSwipeTip())
                        .listRowBackground(Color.clear)
                        .listRowInsets(EdgeInsets())
                }
            } header: {
                // One line on the narrowest phone: the title never wraps, the
                // quarter note gives way first.
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(t("Assignments")).font(.title3.bold()).foregroundStyle(Theme.ink)
                        .lineLimit(1)
                        .fixedSize()
                    Text(t("{{quarter}} · Unsubmitted only", ["quarter": Format.quarter(board.term)]))
                        .font(.footnote)
                        .foregroundStyle(Theme.muted)
                        .lineLimit(1)
                    Spacer(minLength: 4)
                    Button {
                        push(AssignmentRecordRoute())
                    } label: {
                        Text(t("See all")).font(.subheadline.weight(.semibold))
                    }
                    .fixedSize()
                }
                .textCase(nil)
                .padding(.top, 8)
            }
            group(t("Pinned"), board.today.pinned)
            group(t("This week"), board.today.dueThisWeek)
            group(t("Next week and later"), board.today.later)
            group(t("Overdue"), board.today.pastRecent)
        }
    }

    @ViewBuilder
    private func group(_ title: String, _ items: [DeadlineItem]) -> some View {
        if !items.isEmpty {
            Section {
                ForEach(items, id: \.assignmentId) { item in
                    AssignmentListRow(item: item) {
                        DeadlineRow(item: item, now: now, overdueStyle: .elapsed)
                    }
                }
            } header: {
                Text(title)
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(Theme.muted)
                    .textCase(nil)
            }
        }
    }
}

extension DeadlineGroups {
    /// Nothing left to do among the groups 今日 shows.
    var isOpenEmpty: Bool {
        pinned.isEmpty && dueThisWeek.isEmpty && later.isEmpty && undated.isEmpty && pastRecent.isEmpty
    }
}

/// Every assignment of the quarter whatever its state (`AssignmentBoard.record`);
/// older past work stays folded, hidden work comes back from here.
struct RecordScreen: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        List {
            if let record = model.board?.record {
                RecordSections(record: record)
            }
        }
        .listStyle(.insetGrouped)
        .scrollContentBackground(.hidden)
        .background(Theme.bg)
        .navigationTitle(t("All assignments"))
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.visible, for: .navigationBar)
        .onAppear { model.syncHint(.open(view: .assignments)) }
    }
}

/// A quarter's record as list sections, for すべての課題 and 授業's 課題. With
/// `selectable`, rows are tags of the list's selection, which opens them
/// beside it, instead of buttons that push.
struct RecordSections: View {
    let record: DeadlineGroups
    var selectable = false
    @State private var unfolded = false

    var body: some View {
        group(t("Pinned"), record.pinned)
        group(t("This week"), record.dueThisWeek)
        group(t("Next week and later"), record.later)
        group(t("No deadline"), record.undated)
        group(t("Past"), record.pastRecent)
        if !record.pastEarlier.isEmpty {
            if unfolded {
                group(t("Earlier"), record.pastEarlier)
            } else {
                Section {
                    Button(t("Show {{count}} earlier", ["count": record.pastEarlier.count])) { unfolded = true }
                }
            }
        }
        group(t("Hidden"), record.hidden, hidden: true)
    }

    @ViewBuilder
    private func group(_ title: String, _ items: [DeadlineItem], hidden: Bool = false) -> some View {
        if !items.isEmpty {
            Section(title) {
                ForEach(items, id: \.assignmentId) { item in
                    if selectable {
                        DeadlineRow(item: item, now: Date()).tag(item.assignmentId)
                    } else {
                        AssignmentListRow(item: item, hidden: hidden) { DeadlineRow(item: item, now: Date()) }
                    }
                }
            }
        }
    }
}

/// One assignment in a 課題 list (今日 or the record). A plain button rather than a NavigationLink,
/// so List draws no disclosure chevron, and with the system row highlight
/// rather than `PressableStyle`, whose dip on touch-down fights the swipe.
/// Swipe right pins (full swipe allowed); swipe left hides, never by a long
/// pull alone. A hidden row offers only 戻す.
struct AssignmentListRow<Content: View>: View {
    let item: DeadlineItem
    var hidden = false
    @ViewBuilder var label: Content
    @Environment(AppModel.self) private var model
    @Environment(\.push) private var push
    /// Bumped by every pin or hide, so each one answers with a tap.
    @State private var acted = 0

    var body: some View {
        Button {
            push(AssignmentRoute(id: item.assignmentId))
        } label: {
            label
        }
        .listRowInsets(EdgeInsets())
        .contextMenu {
            if let course = item.courseId {
                Button(t("Open course"), systemImage: "book") { push(CourseRoute(id: course)) }
            }
            if let url = item.sourceUrl {
                Button(t("Submit in LMS"), systemImage: "arrow.up.forward.square") {
                    push(WebRouter.PushedPage(site: .scienceLms, target: url, title: item.title))
                }
            }
            if hidden {
                Button(t("Restore"), systemImage: "eye") { setHidden(false) }
            } else {
                Button(item.pinned ? t("Unpin") : t("Pin"), systemImage: "pin") { togglePin() }
                Button(t("Hide"), systemImage: "eye.slash", role: .destructive) { setHidden(true) }
            }
        }
        .swipeActions(edge: .leading) {
            if !hidden {
                Button {
                    togglePin()
                } label: {
                    Label(item.pinned ? t("Unpin") : t("Pin"), systemImage: item.pinned ? "pin.slash" : "pin")
                }
                .tint(Theme.accent)
            }
        }
        .swipeActions(edge: .trailing, allowsFullSwipe: hidden) {
            if hidden {
                Button { setHidden(false) } label: { Label(t("Restore"), systemImage: "eye") }
                    .tint(Theme.accent)
            } else {
                Button { setHidden(true) } label: { Label(t("Hide"), systemImage: "eye.slash") }
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

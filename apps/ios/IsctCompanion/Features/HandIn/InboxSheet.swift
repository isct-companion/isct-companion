import IsctCore
import SwiftUI

/// The upload panel: what waits in the inbox, then the quarter's unsubmitted
/// assignments as 今日 groups them. Picking one puts the files in its upload
/// card and opens its page, where 「LMS にアップロード」 is the next step.
struct InboxSheet: View {
    @Environment(InboxModel.self) private var inbox
    @Environment(AppModel.self) private var model
    @Environment(Navigator.self) private var navigator
    @Environment(\.dismiss) private var dismiss
    @State private var giving = false

    var body: some View {
        NavigationStack {
            List {
                Group {
                    Section {
                        ScrollView(.horizontal, showsIndicators: false) {
                            HStack(spacing: Theme.space6) {
                                ForEach(inbox.items) { item in
                                    VStack(spacing: Theme.space3) {
                                        if let url = item.url { Thumbnail(url: url).frame(width: 72, height: 92) }
                                        Text(item.name).font(.caption2).lineLimit(1).frame(width: 80)
                                    }
                                    .contextMenu {
                                        Button(t("common.delete"), systemImage: "trash", role: .destructive) { inbox.remove(item) }
                                    }
                                }
                            }
                            .padding(.vertical, Theme.space2)
                        }
                    }
                    if let groups = model.board?.today {
                        section(t("common.pinned"), groups.pinned)
                        section(t("common.thisWeek"), groups.dueThisWeek)
                        section(t("assignments.nextWeekAndLater"), groups.later)
                        section(t("common.noDeadline"), groups.undated)
                        section(t("common.past"), groups.pastRecent)
                    }
                }
                .themedRows()
            }
            .themedList()
            .disabled(giving)
            .navigationTitle(t("common.handInTo"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button(t("common.close")) { dismiss() } }
            }
            .onChange(of: inbox.items.isEmpty) { _, empty in if empty { dismiss() } }
        }
        .presentationDetents([.large])
    }

    @ViewBuilder
    private func section(_ title: String, _ items: [DeadlineItem]) -> some View {
        if !items.isEmpty {
            Section(title) {
                ForEach(items, id: \.assignmentId) { item in
                    Button { give(to: item.assignmentId) } label: { DeadlineRow(item: item, now: Date()) }
                        .buttonStyle(.plain)
                }
            }
        }
    }

    private func give(to assignment: String) {
        giving = true
        Task {
            defer { giving = false }
            if await inbox.give(inbox.items, to: assignment, model) {
                dismiss()
                navigator.tab = .today
                navigator.open(AssignmentRoute(id: assignment))
            }
        }
    }
}

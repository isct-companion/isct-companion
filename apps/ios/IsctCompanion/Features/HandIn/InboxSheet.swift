import IsctCore
import SwiftUI

/// The selected files and the assignments that can receive them now.
struct InboxSheet: View {
    let selection: SharedInbox.Transfer
    @Environment(InboxModel.self) private var inbox
    @Environment(AppModel.self) private var model
    @Environment(Navigator.self) private var navigator
    @State private var started = false
    @State private var error: String?

    var body: some View {
        NavigationStack {
            List {
                Group {
                    Section {
                        ScrollView(.horizontal, showsIndicators: false) {
                            HStack(spacing: Theme.space6) {
                                ForEach(selection.files) { file in
                                    VStack(spacing: Theme.space3) {
                                        Thumbnail(url: file.url(in: selection.folder)).frame(width: 72, height: 92)
                                        Text(file.name).font(.caption2).lineLimit(1).frame(width: 80)
                                    }
                                }
                            }
                            .padding(.vertical, Theme.space2)
                        }
                    }
                    if inbox.working { ProgressView() }
                    if let error { Text(error).foregroundStyle(Theme.danger) }
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
            .navigationTitle(t("common.handInTo"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(t("common.cancel")) { inbox.discard(selection) }
                }
            }
            .onChange(of: inbox.working, initial: true) { _, working in
                guard !working, !started else { return }
                started = true
                if let choice = selection.choice { give(to: choice.id, title: choice.title) }
            }
        }
        .interactiveDismissDisabled()
        .presentationDetents([.large])
    }

    @ViewBuilder
    private func section(_ title: String, _ items: [DeadlineItem]) -> some View {
        let available = items.filter { $0.handInAvailability == .prepareDraft }
        if !available.isEmpty {
            Section(title) {
                ForEach(available, id: \.assignmentId) { item in
                    Button { give(to: item.assignmentId, title: item.title) } label: { DeadlineRow(item: item, now: AppClock.now) }
                        .buttonStyle(.plain)
                        .disabled(inbox.working)
                }
            }
        }
    }

    private func give(to assignment: String, title: String) {
        error = nil
        Task {
            do {
                if let page = try await inbox.give(selection, to: assignment, title: title, model: model) {
                    navigator.push(page)
                }
            } catch {
                self.error = AppModel.describe(error)
                await model.refresh()
            }
        }
    }
}

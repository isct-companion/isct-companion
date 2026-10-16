import IsctCore
import SwiftUI

/// 課題: open assignments grouped by Core (`AssignmentBoard`). Swipe right
/// pins, swipe left hides; submitted and hidden work sits behind one row.
struct AssignmentsScreen: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        TimelineView(.periodic(from: .now, by: 60)) { context in
            List {
                Section {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("\(Format.quarter(model.board?.term ?? "")) · 未提出のみ")
                            .font(.subheadline)
                            .foregroundStyle(Theme.muted)
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
                if let board = model.board {
                    group("ピン留め", board.pinned, now: context.date)
                    group("期限切れ", board.overdue, now: context.date)
                    group("今日", board.today, now: context.date)
                    group("今週", board.thisWeek, now: context.date)
                    group("来週以降", board.later, now: context.date)
                    if isEmpty(board) {
                        Section {
                            ContentUnavailableView("未提出の課題はありません", systemImage: "checkmark.circle")
                        }
                        .listRowBackground(Color.clear)
                    }
                    Section {
                        NavigationLink {
                            SubmittedScreen()
                        } label: {
                            Text("提出済・非表示").foregroundStyle(Theme.ink)
                        }
                    }
                }
            }
            .listStyle(.insetGrouped)
            .scrollContentBackground(.hidden)
            .background(Theme.bg)
            .refreshable { await model.pull() }
        }
        .toolbar(.hidden, for: .navigationBar)
        .safeAreaInset(edge: .bottom) { UndoToast() }
    }

    private func isEmpty(_ board: AssignmentBoard) -> Bool {
        board.pinned.isEmpty && board.overdue.isEmpty && board.today.isEmpty && board.thisWeek.isEmpty
            && board.later.isEmpty
    }

    @ViewBuilder
    private func group(_ title: LocalizedStringKey, _ items: [DeadlineItem], now: Date) -> some View {
        if !items.isEmpty {
            Section(title) {
                ForEach(items, id: \.assignmentId) { item in
                    DeadlineLink(item: item) {
                        DeadlineRow(item: item, now: now, overdueStyle: .elapsed)
                    }
                    .listRowInsets(EdgeInsets())
                    .swipeActions(edge: .leading) {
                        Button {
                            Task { await model.setPinned(item, !item.pinned) }
                        } label: {
                            Label(item.pinned ? "外す" : "ピン留め", systemImage: item.pinned ? "pin.slash" : "pin")
                        }
                        .tint(Theme.accent)
                    }
                    .swipeActions(edge: .trailing) {
                        Button {
                            Task { await model.setHidden(item, true) }
                        } label: {
                            Label("非表示", systemImage: "eye.slash")
                        }
                        .tint(Theme.muted)
                    }
                }
            }
        }
    }
}

/// Submitted work and hidden assignments; hidden ones come back from here.
private struct SubmittedScreen: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        List {
            if let board = model.board {
                if !board.hidden.isEmpty {
                    Section("非表示") {
                        ForEach(board.hidden, id: \.assignmentId) { item in
                            DeadlineRow(item: item, now: Date())
                                .listRowInsets(EdgeInsets())
                                .swipeActions(edge: .trailing) {
                                    Button {
                                        Task { await model.setHidden(item, false) }
                                    } label: {
                                        Label("戻す", systemImage: "eye")
                                    }
                                    .tint(Theme.accent)
                                }
                        }
                    }
                }
                if !board.submitted.isEmpty {
                    Section("提出済") {
                        ForEach(board.submitted, id: \.assignmentId) { item in
                            DeadlineLink(item: item) { DeadlineRow(item: item, now: Date()) }
                                .listRowInsets(EdgeInsets())
                        }
                    }
                }
            }
        }
        .listStyle(.insetGrouped)
        .scrollContentBackground(.hidden)
        .background(Theme.bg)
        .navigationTitle("提出済・非表示")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.visible, for: .navigationBar)
    }
}

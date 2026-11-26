import SwiftUI

extension View {
    /// The hide-undo bar over a page's bottom edge. `AppModel.undo` holds the
    /// one pending undo, so the shell attaches this once per visible page
    /// area (each tab's stack, the workstation's page) and every screen that
    /// hides an assignment (今日, 課題, すべての課題, an assignment) offers it.
    func undoToast() -> some View {
        safeAreaInset(edge: .bottom) { UndoToast() }
    }
}

private struct UndoToast: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if let undo = model.undo {
            HStack {
                Text(t("app.hidTitle", ["title": undo.title])).lineLimit(1)
                Spacer()
                Button(t("common.undo")) { Task { await model.undoHide() } }.fontWeight(.semibold)
            }
            .font(.subheadline)
            .foregroundStyle(Theme.ink)
            .tint(Theme.accent)
            .padding(.horizontal, Theme.space8)
            .padding(.vertical, Theme.space6)
            // A floating layer takes the system material, not a black pill.
            .floatingGlass(in: Capsule())
            .padding(.horizontal, Theme.gutter)
            .padding(.bottom, Theme.space4)
            .transition(.move(edge: .bottom).combined(with: .opacity))
            .task(id: undo) {
                try? await Task.sleep(for: .seconds(4))
                model.dismissUndo()
            }
        }
    }
}

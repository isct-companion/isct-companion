import IsctCore
import SwiftUI
import UIKit

/// Text metrics of a timetable block. Columns are about 60 pt wide, so the
/// title takes every line the block's height leaves above the room line.
enum BlockText {
    static let padding: CGFloat = 4
    static let spacing: CGFloat = 2
    /// `.caption2`, not `.caption`: SwiftUI's Text ignores a paragraph style's
    /// `hyphenationFactor`, so a smaller face is what keeps English words
    /// ("Assembly") whole in a ~60 pt column more often.
    static let titleFont = Font.caption2.weight(.semibold)
    static let roomFont = Font.caption2.weight(.semibold)

    /// Title lines that fit above the room line, at least one.
    static func titleLines(height: CGFloat) -> Int {
        let line = UIFont.preferredFont(forTextStyle: .caption2).lineHeight
        return max(1, Int(((height - line - 2 * padding - spacing) / line).rounded(.down)))
    }

    /// The first of several rooms joined by commas.
    static func firstRoom(_ room: String) -> String {
        room.split(whereSeparator: { ",、，".contains($0) }).first
            .map { $0.trimmingCharacters(in: .whitespaces) } ?? room
    }
}

/// Long press on a class or exam: mark this one session as an exam, 休講 or
/// a normal class (Core's `MarkSession`); the current state is checked.
private struct SessionMarks: ViewModifier {
    let block: Occurrence
    @Environment(AppModel.self) private var model

    func body(content: Content) -> some View {
        content.contextMenu {
            Picker(t("This lesson"), selection: Binding(get: { state }, set: { mark($0) })) {
                Text(t("Mark as exam")).tag("exam")
                Text(t("Mark as cancelled")).tag("cancelled")
                Text(t("Back to a regular class")).tag("class")
            }
            .pickerStyle(.inline)
        }
    }

    private var state: String {
        block.cancelled == true ? "cancelled" : block.kind == .exam ? "exam" : "class"
    }

    private func mark(_ value: String) {
        guard value != state else { return }
        Task { await model.markSession(block.key, value) }
    }
}

extension View {
    @ViewBuilder
    func sessionMarks(_ block: Occurrence, enabled: Bool) -> some View {
        if enabled { modifier(SessionMarks(block: block)) } else { self }
    }
}

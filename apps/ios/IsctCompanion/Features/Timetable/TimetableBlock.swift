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

/// 試験 before an exam block's room, solid in the course colour; a quiz's
/// 小テスト is outlined (design/README.md).
struct ExamTag: View {
    let quiz: Bool
    let tint: Color

    var body: some View {
        let shape = Capsule()
        Text(quiz ? t("Quiz") : t("Exam"))
            // The course palette is mid-tone in both themes.
            .foregroundStyle(quiz ? tint : .white)
            .padding(.horizontal, 4)
            .background { if !quiz { shape.fill(tint) } }
            .overlay { if quiz { shape.strokeBorder(tint, lineWidth: 1) } }
            .fixedSize()
    }
}

/// Long press on a class or exam: mark this one session as an exam, a quiz
/// or 休講 (Core's `MarkSession`). The current mark is checked; choosing it
/// again sends `class`, and Core decides whether that clears the mark.
private struct SessionMarks: ViewModifier {
    let block: Occurrence
    @Environment(AppModel.self) private var model

    private var marks: [(String, String)] {
        [("exam", t("Mark as exam")), ("quiz", t("Mark as quiz")), ("cancelled", t("Mark as cancelled"))]
    }

    func body(content: Content) -> some View {
        content.contextMenu {
            Section(t("This lesson")) {
                ForEach(marks, id: \.0) { value, label in
                    Toggle(label, isOn: Binding(get: { state == value }, set: { _ in mark(value) }))
                }
            }
        }
    }

    private var state: String {
        if block.cancelled == true { return "cancelled" }
        guard block.kind == .exam else { return "class" }
        return block.quiz == true ? "quiz" : "exam"
    }

    private func mark(_ value: String) {
        let next = value == state ? "class" : value
        Task { await model.markSession(block.key, next) }
    }
}

extension View {
    @ViewBuilder
    func sessionMarks(_ block: Occurrence, enabled: Bool) -> some View {
        if enabled { modifier(SessionMarks(block: block)) } else { self }
    }
}

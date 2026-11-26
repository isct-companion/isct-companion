import IsctCore
import SwiftUI
import UIKit

/// Text metrics of a timetable block. Columns are about 60 pt wide, so the
/// title takes every line the block's height leaves above the room line.
enum BlockText {
    static let padding: CGFloat = Theme.space2
    static let spacing: CGFloat = Theme.space1
    /// `.caption2`, not `.caption`: SwiftUI's Text ignores a paragraph style's
    /// `hyphenationFactor`, so a smaller face is what keeps English words
    /// ("Assembly") whole in a ~60 pt column more often.
    static let titleFont = Font.block
    static let roomFont = Font.block

    /// Title lines that fit beside the tag line above and the room line
    /// below, at least one.
    static func titleLines(height: CGFloat, tagged: Bool, footer: Bool) -> Int {
        let line = UIFont.preferredFont(forTextStyle: .caption2).lineHeight
        let taken = CGFloat((tagged ? 1 : 0) + (footer ? 1 : 0))
        return max(1, Int(((height - taken * (line + spacing) - 2 * padding) / line).rounded(.down)))
    }

    /// The first of several rooms joined by commas.
    static func firstRoom(_ room: String) -> String {
        room.split(whereSeparator: { ",、，".contains($0) }).first
            .map { $0.trimmingCharacters(in: .whitespaces) } ?? room
    }
}

/// 試験 above an exam block's title, solid in the course colour; a quiz's
/// 小テスト is outlined (slotledger `approach/timetable-exam-emphasis`).
struct ExamTag: View {
    let quiz: Bool
    let tint: Color

    var body: some View {
        Tag(text: quiz ? t("common.quiz") : t("common.exam"), style: quiz ? .outline : .solid,
            ink: quiz ? Theme.courseInk(tint) : tint)
    }
}

/// Long press on a class or exam: mark this one session as an exam, a quiz
/// or 休講 (Core's `MarkSession`). Core's `Occurrence.mark` is checked;
/// choosing it again sends `class`, and Core decides whether that clears the
/// mark.
private struct SessionMarks: ViewModifier {
    let block: Occurrence
    @Environment(AppModel.self) private var model

    private var marks: [(SessionMark, String, String)] {
        [(.exam, "exam", t("common.markAsExam")), (.quiz, "quiz", t("common.markAsQuiz")),
         (.cancelled, "cancelled", t("common.markAsCancelled"))]
    }

    func body(content: Content) -> some View {
        content.contextMenu {
            Section(t("timetable.thisLesson")) {
                ForEach(marks, id: \.1) { mark, value, label in
                    Toggle(label, isOn: Binding(
                        get: { block.mark == mark },
                        set: { _ in
                            let next = block.mark == mark ? "class" : value
                            Task { await model.markSession(block.key, next) }
                        }
                    ))
                }
            }
        }
    }
}

extension View {
    @ViewBuilder
    /// Core leaves `mark` empty on a block that takes none.
    func sessionMarks(_ block: Occurrence) -> some View {
        if block.mark != nil { modifier(SessionMarks(block: block)) } else { self }
    }
}

extension TimetableScreen {
    /// One block (slotledger `approach/timetable-*`): a solid course fill,
    /// deeper for an exam; a ring only for the class in progress; an exam or
    /// quiz tag above the title; a class that does not take place striped.
    func blockView(_ block: Occurrence, height: CGFloat, narrow: Bool) -> some View {
        let planned = block.planned == true
        let cancelled = block.cancelled == true
        let exam = block.kind == .exam && !cancelled
        let quiz = exam && block.quiz == true
        let current = isCurrent(block)
        let tint = Color(hex: block.color)
        let shape = RoundedRectangle(cornerRadius: Theme.radiusBlock, style: .continuous)
        let tagged = exam && !narrow
        let content = VStack(alignment: .leading, spacing: BlockText.spacing) {
            if tagged {
                HStack(spacing: 3) {
                    ExamTag(quiz: quiz, tint: tint)
                    if block.sourceChanged == true {
                        Image(systemName: "exclamationmark.circle.fill")
                            .foregroundStyle(Theme.warn)
                            .accessibilityLabel(t("common.timetableChanged"))
                    }
                }
                .font(BlockText.titleFont)
            }
            Text(block.title)
                .strikethrough(cancelled)
                .font(BlockText.titleFont)
                .lineLimit(BlockText.titleLines(height: height, tagged: tagged, footer: !narrow))
            if !narrow {
                Spacer(minLength: 0)
                bottomLine(block, planned: planned, cancelled: cancelled)
                    .font(BlockText.roomFont)
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
            }
        }
        .foregroundStyle(cancelled ? Theme.muted : Theme.courseInk(tint))
        .padding(BlockText.padding)
        .frame(maxWidth: .infinity, alignment: .topLeading)
        .frame(height: height)
        .background {
            if cancelled {
                shape.fill(Theme.mix(Theme.muted, 0.10, into: Theme.surface))
                    .overlay { Stripes().clipShape(shape) }
            } else if planned {
                shape.fill(Theme.surface)
            } else {
                shape.fill(Theme.mix(tint, exam && !quiz ? Theme.tintExam : Theme.tintFill, into: Theme.surface))
            }
        }
        .overlay {
            if current && !cancelled {
                shape.stroke(tint, lineWidth: 2)
            } else if planned && !cancelled {
                shape.stroke(Theme.mix(tint, 0.45, into: Theme.surface), lineWidth: 1.5)
            }
        }
        .contentShape(shape)
        .padding(.horizontal, Theme.space1)
        return Group {
            if let id = block.courseId {
                NavigationLink(value: CourseRoute(id: id, zoom: block.key)) { content }
                    .buttonStyle(PressableStyle())
                    .courseZoomSource(block.key)
            } else {
                content
            }
        }
        .sessionMarks(block)
    }

    /// 計画, 休講, the real times of a block drawn over whole periods, or the
    /// first room; every room stays on the course page.
    @ViewBuilder
    private func bottomLine(_ block: Occurrence, planned: Bool, cancelled: Bool) -> some View {
        if planned {
            Text(t("timetable.planned"))
        } else if cancelled {
            Text(t("common.noClass"))
        } else if let grid = block.grid,
            grid.startTime != block.startTime || grid.endTime != block.endTime
        {
            Text("\(Self.clock(block.startTime))–\(Self.clock(block.endTime))").monospacedDigit()
        } else {
            Text(BlockText.firstRoom(block.room))
        }
    }

    /// "09:30" as 9:30, as the period column writes times.
    private static func clock(_ time: String) -> String {
        time.replacingOccurrences(of: "^0", with: "", options: .regularExpression)
    }
}

/// Diagonal stripes on a class that does not take place.
private struct Stripes: View {
    var body: some View {
        Canvas { context, size in
            var path = Path()
            var x: CGFloat = -size.height
            while x < size.width {
                path.move(to: CGPoint(x: x, y: size.height))
                path.addLine(to: CGPoint(x: x + size.height, y: 0))
                x += 8
            }
            context.stroke(path, with: .color(Theme.line), lineWidth: 3)
        }
        .allowsHitTesting(false)
    }
}

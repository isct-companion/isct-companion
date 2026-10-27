import SwiftUI
import UIKit

/// Adaptive roles over the generated light/dark tokens (design/tokens). System
/// chrome (tab bar, navigation, sheets, controls) stays the platform's.
enum Theme {
    static let bg = adaptive(IsctTokens.colorBg, IsctTokensDark.colorBg)
    static let surface = adaptive(IsctTokens.colorSurface, IsctTokensDark.colorSurface)
    static let surface2 = adaptive(IsctTokens.colorSurface2, IsctTokensDark.colorSurface2)
    static let ink = adaptive(IsctTokens.colorInk, IsctTokensDark.colorInk)
    static let muted = adaptive(IsctTokens.colorMuted, IsctTokensDark.colorMuted)
    static let line = adaptive(IsctTokens.colorLine, IsctTokensDark.colorLine)
    static let accent = adaptive(IsctTokens.colorAccent, IsctTokensDark.colorAccent)
    static let accentSoft = adaptive(IsctTokens.colorAccentSoft, IsctTokensDark.colorAccentSoft)
    /// Text on the accent fill (a primary button, today's date).
    static let accentInk = adaptive(IsctTokens.colorAccentInk, IsctTokensDark.colorAccentInk)
    /// Text on the solid course colour (an exam tag).
    static let onSolid = IsctTokens.courseOnSolid
    static let danger = adaptive(IsctTokens.colorDanger, IsctTokensDark.colorDanger)
    static let dangerSoft = adaptive(IsctTokens.colorDangerSoft, IsctTokensDark.colorDangerSoft)
    static let warn = adaptive(IsctTokens.colorWarn, IsctTokensDark.colorWarn)
    static let warnSoft = adaptive(IsctTokens.colorWarnSoft, IsctTokensDark.colorWarnSoft)
    static let ok = adaptive(IsctTokens.colorOk, IsctTokensDark.colorOk)
    static let okSoft = adaptive(IsctTokens.colorOkSoft, IsctTokensDark.colorOkSoft)

    static let radiusSurface = IsctTokens.radiusSurface
    static let radiusPanel = IsctTokens.radiusPanel
    static let radiusChip = IsctTokens.radiusChip
    static let radiusBlock = IsctTokens.radiusBlock
    static let radiusColumn = IsctTokens.radiusColumn

    /// Page side margin, list row padding and gap, card padding.
    static let gutter = IsctTokens.spaceGutter
    static let rowX = IsctTokens.spaceRowX
    static let rowY = IsctTokens.spaceRowY
    static let rowGap = IsctTokens.spaceRowGap
    static let cardPadding = IsctTokens.spaceCard

    /// Course colour strength of a block that occupies time, a quiz's
    /// included; an exam's deeper fill; text on a fill.
    static let tintFill = IsctTokens.courseTintFill
    static let tintExam = IsctTokens.courseTintExam
    static let tintInk = IsctTokens.courseTintInk

    /// The course colour mixed into `base` as a solid colour, as desktop's
    /// `color-mix`, so rules and bands under a block never show through.
    static func mix(_ course: Color, _ share: Double, into base: Color) -> Color {
        base.mix(with: course, by: share, in: .device)
    }

    /// Text on a course fill: the course colour mixed into ink.
    static func courseInk(_ course: Color) -> Color { mix(course, tintInk, into: ink) }

    private static func adaptive(_ light: Color, _ dark: Color) -> Color {
        let light = UIColor(light)
        let dark = UIColor(dark)
        return Color(UIColor { $0.userInterfaceStyle == .dark ? dark : light })
    }
}

extension Color {
    /// Course colours arrive from Core as `#rrggbb`.
    init(hex: String?) {
        let digits = (hex ?? "").trimmingCharacters(in: CharacterSet(charactersIn: "#"))
        guard digits.count == 6, let value = UInt32(digits, radix: 16) else {
            self = Theme.accent
            return
        }
        self.init(
            red: Double((value >> 16) & 0xff) / 255,
            green: Double((value >> 8) & 0xff) / 255,
            blue: Double(value & 0xff) / 255
        )
    }
}

/// White rounded card on the warm background, the phone's one container.
struct Card<Content: View>: View {
    var padding: CGFloat = 0
    @ViewBuilder var content: Content
    var body: some View {
        VStack(alignment: .leading, spacing: 0) { content }
            .padding(padding)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Theme.surface, in: RoundedRectangle(cornerRadius: Theme.radiusSurface, style: .continuous))
    }
}

/// Text roles (design/tokens `type`), mapped to system styles so Dynamic
/// Type applies.
extension Font {
    /// Hero figures; the desktop's 40.
    static var display: Font {
        .system(size: UIFontMetrics(forTextStyle: .largeTitle).scaledValue(for: 40), weight: .bold)
    }
    static let pageTitle = Font.largeTitle.bold()
    static let section = Font.subheadline.weight(.semibold)
    static let rowTitle = Font.body
    static let note = Font.footnote
    /// Timetable block text and tags.
    static let block = Font.caption2.weight(.semibold)
    /// Time ticks beside the timetable.
    static let tick = Font.caption2
}

/// Small grey caption above a card.
struct SectionLabel: View {
    let text: String
    init(_ text: String) { self.text = text }
    var body: some View {
        Text(text)
            .font(.section)
            .foregroundStyle(Theme.muted)
            .padding(.horizontal, Theme.rowX)
            .padding(.top, 8)
    }
}

/// "Belongs to this course" in lists.
struct CourseSquare: View {
    let color: String?
    var body: some View {
        RoundedRectangle(cornerRadius: 3, style: .continuous)
            .fill(Color(hex: color))
            .frame(width: 10, height: 10)
    }
}

/// Thin divider inset past the leading marker, as in grouped lists.
/// A row inside a `Card` that pushes `route`. In a List a nested
/// NavigationLink gets its disclosure chevron drawn outside the card, so the
/// row is a button that draws the chevron itself, 20pt from the card's edge as
/// a Settings row has it.
struct CardLink<Route: Hashable, Content: View>: View {
    let route: Route?
    /// Runs as the row opens.
    var opened: (() -> Void)?
    @ViewBuilder var content: Content
    @Environment(\.push) private var push

    var body: some View {
        if let route {
            Button {
                opened?()
                push(route)
            } label: {
                HStack(spacing: Theme.rowGap) {
                    content
                    Image(systemName: "chevron.forward")
                        .font(.system(size: 14, weight: .semibold))
                        .foregroundStyle(Color(uiColor: .tertiaryLabel))
                }
                .padding(.leading, Theme.rowX)
                .padding(.trailing, 20)
                .padding(.vertical, Theme.rowY)
                .contentShape(Rectangle())
            }
            .buttonStyle(PressableStyle())
        } else {
            content
                .padding(.horizontal, Theme.rowX)
                .padding(.vertical, Theme.rowY)
        }
    }
}

struct RowDivider: View {
    var inset: CGFloat = 40
    var body: some View {
        Rectangle().fill(Theme.line).frame(height: 0.5).padding(.leading, inset)
    }
}

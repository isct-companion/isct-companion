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
    static let danger = adaptive(IsctTokens.colorDanger, IsctTokensDark.colorDanger)
    static let dangerSoft = adaptive(IsctTokens.colorDangerSoft, IsctTokensDark.colorDangerSoft)
    static let warn = adaptive(IsctTokens.colorWarn, IsctTokensDark.colorWarn)
    static let warnSoft = adaptive(IsctTokens.colorWarnSoft, IsctTokensDark.colorWarnSoft)
    static let ok = adaptive(IsctTokens.colorOk, IsctTokensDark.colorOk)
    static let okSoft = adaptive(IsctTokens.colorOkSoft, IsctTokensDark.colorOkSoft)

    static let radiusSurface = IsctTokens.radiusSurface
    static let radiusPanel = IsctTokens.radiusPanel
    static let radiusChip = IsctTokens.radiusChip
    /// Course colour strengths: a block that occupies time, and row emphasis.
    static let tintFill = IsctTokens.courseTintFill
    static let tintSoft = IsctTokens.courseTintSoft

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

/// Small grey caption above a card.
struct SectionLabel: View {
    let text: String
    init(_ text: String) { self.text = text }
    var body: some View {
        Text(text)
            .font(.footnote.weight(.semibold))
            .foregroundStyle(Theme.muted)
            .padding(.horizontal, 16)
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
struct RowDivider: View {
    var inset: CGFloat = 40
    var body: some View {
        Rectangle().fill(Theme.line).frame(height: 0.5).padding(.leading, inset)
    }
}

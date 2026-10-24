import SwiftUI
import UIKit

// The finish the content components share (docs/ux/mobile.md, Look). Built
// on SwiftUI's own animation APIs, so Reduce Motion and Dynamic Type apply.

/// The 'now' tint desktop draws on Today's current or next class and Android
/// on its hero card (design/README.md): the course hue at the theme's hero
/// lightness, chroma capped (design/tokens `hero`), fading slightly from top
/// to bottom. Mixing the raw course colour into the surface greyed muted
/// courses, and a corner gradient read as lopsided.
struct CourseTint: View {
    let tint: Color
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        let dark = scheme == .dark
        let traits = UITraitCollection(userInterfaceStyle: dark ? .dark : .light)
        let top = OKLab(UIColor(tint).resolvedColor(with: traits)).hero(
            lightness: dark ? IsctTokensDark.heroLightness : IsctTokens.heroLightness,
            chroma: dark ? IsctTokensDark.heroChroma : IsctTokens.heroChroma
        )
        let surface = OKLab(UIColor(Theme.surface).resolvedColor(with: traits))
        let fade = dark ? IsctTokensDark.heroFade : IsctTokens.heroFade
        LinearGradient(
            colors: [top.color, surface.mix(top, fade).color],
            startPoint: .top, endPoint: .bottom
        )
    }
}

/// A colour in OKLab, for the hero tint and its fade (desktop uses CSS
/// `oklch(from …)` and `color-mix(in oklab)`, Android Compose's Oklab).
private struct OKLab {
    var l: Double, a: Double, b: Double

    init(l: Double, a: Double, b: Double) { (self.l, self.a, self.b) = (l, a, b) }

    init(_ color: UIColor) {
        var r: CGFloat = 0, g: CGFloat = 0, bl: CGFloat = 0, alpha: CGFloat = 0
        color.getRed(&r, green: &g, blue: &bl, alpha: &alpha)
        func linear(_ c: CGFloat) -> Double {
            let c = Double(c)
            return c <= 0.04045 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4)
        }
        let (lr, lg, lb) = (linear(r), linear(g), linear(bl))
        let lc = cbrt(0.4122214708 * lr + 0.5363325363 * lg + 0.0514459929 * lb)
        let mc = cbrt(0.2119034982 * lr + 0.6806995451 * lg + 0.1073969566 * lb)
        let sc = cbrt(0.0883024619 * lr + 0.2817188376 * lg + 0.6299787005 * lb)
        self.init(
            l: 0.2104542553 * lc + 0.7936177850 * mc - 0.0040720468 * sc,
            a: 1.9779984951 * lc - 2.4285922050 * mc + 0.4505937099 * sc,
            b: 0.0259040371 * lc + 0.7827717662 * mc - 0.8086757660 * sc
        )
    }

    /// The same hue at `lightness`, its chroma no higher than `chroma`.
    func hero(lightness: Double, chroma: Double) -> OKLab {
        let current = (a * a + b * b).squareRoot()
        let scale = current > chroma ? chroma / current : 1
        return OKLab(l: lightness, a: a * scale, b: b * scale)
    }

    /// `share` of `other` mixed into this colour.
    func mix(_ other: OKLab, _ share: Double) -> OKLab {
        OKLab(l: l + (other.l - l) * share, a: a + (other.a - a) * share, b: b + (other.b - b) * share)
    }

    var color: Color {
        let lc = pow(l + 0.3963377774 * a + 0.2158037573 * b, 3)
        let mc = pow(l - 0.1055613458 * a - 0.0638541728 * b, 3)
        let sc = pow(l - 0.0894841775 * a - 1.2914855480 * b, 3)
        func clamp(_ v: Double) -> Double { min(max(v, 0), 1) }
        return Color(
            .sRGBLinear,
            red: clamp(4.0767416621 * lc - 3.3077115913 * mc + 0.2309699292 * sc),
            green: clamp(-1.2684380046 * lc + 2.6097574011 * mc - 0.3413193965 * sc),
            blue: clamp(-0.0041960863 * lc - 0.7034186147 * mc + 1.7076147010 * sc)
        )
    }
}

extension View {
    /// The hero card: course tint, continuous corners and a hairline in the tint.
    func heroCard(tint: Color) -> some View {
        let shape = RoundedRectangle(cornerRadius: Theme.radiusPanel, style: .continuous)
        return padding(18)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background { CourseTint(tint: tint).clipShape(shape) }
            .overlay { shape.strokeBorder(tint.opacity(Theme.tintFill), lineWidth: 0.5) }
    }

    /// Figures that count (minutes left, the ring's percentage) roll instead
    /// of jumping.
    func countingFigure<Value: Equatable>(_ value: Value, down: Bool = true) -> some View {
        contentTransition(.numericText(countsDown: down))
            .animation(.snappy, value: value)
    }
}

// MARK: Zoom into a course

private struct CourseZoomKey: EnvironmentKey {
    static let defaultValue: Namespace.ID? = nil
}

extension EnvironmentValues {
    /// The root's namespace for zooming from a block, card or row into its course.
    var courseZoom: Namespace.ID? {
        get { self[CourseZoomKey.self] }
        set { self[CourseZoomKey.self] = newValue }
    }
}

private struct CourseZoomSource: ViewModifier {
    let id: String?
    @Environment(\.courseZoom) private var namespace

    func body(content: Content) -> some View {
        if let id, let namespace {
            content.matchedTransitionSource(id: id, in: namespace)
        } else {
            content
        }
    }
}

private struct CourseZoomDestination: ViewModifier {
    let id: String?
    @Environment(\.courseZoom) private var namespace

    func body(content: Content) -> some View {
        if let id, let namespace {
            content.navigationTransition(.zoom(sourceID: id, in: namespace))
        } else {
            content
        }
    }
}

extension View {
    /// Marks where a `CourseRoute` with the same `zoom` id is opened from.
    func courseZoomSource(_ id: String?) -> some View { modifier(CourseZoomSource(id: id)) }
    func courseZoomDestination(_ id: String?) -> some View { modifier(CourseZoomDestination(id: id)) }
}

// MARK: Glass

extension View {
    /// Our own floating controls take Liquid Glass on iOS 26 and the thin
    /// material before it; system chrome is never drawn by us.
    @ViewBuilder
    func floatingGlass<S: Shape>(in shape: S) -> some View {
        if #available(iOS 26.0, *) {
            glassEffect(.regular, in: shape)
        } else {
            background(.thinMaterial, in: shape)
        }
    }
}

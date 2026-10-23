import SwiftUI

// The finish the content components share (docs/ux/mobile.md, Look). Built
// on SwiftUI's own animation APIs, so Reduce Motion and Dynamic Type apply.

/// The course colour as a soft mesh: the hero card's surface. This is the
/// 'now' gradient desktop draws on Today's current or next class: it peaks
/// at `Theme.tintHero` in the top-leading corner and fades to the surface.
struct CourseMesh: View {
    let tint: Color

    /// Each point's share of the peak, top-leading to bottom-trailing.
    private static let falloff: [Double] = [
        1, 0.6, 0.3,
        0.67, 0.37, 0.17,
        0.37, 0.17, 0.07,
    ]

    var body: some View {
        MeshGradient(
            width: 3, height: 3,
            points: [
                [0, 0], [0.5, 0], [1, 0],
                [0, 0.5], [0.62, 0.42], [1, 0.5],
                [0, 1], [0.5, 1], [1, 1],
            ],
            colors: Self.falloff.map { tint.opacity(Theme.tintHero * $0) }
        )
        .background(Theme.surface)
    }
}

extension View {
    /// The hero card: course mesh, continuous corners and a hairline in the tint.
    func heroCard(tint: Color) -> some View {
        let shape = RoundedRectangle(cornerRadius: Theme.radiusPanel, style: .continuous)
        return padding(18)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background { CourseMesh(tint: tint).clipShape(shape) }
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

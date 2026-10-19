import SwiftUI
import UIKit

/// Which way a page turn goes; callers pick the transition from it.
enum PageStep {
    case previous
    case next
}

/// Horizontal drags over content turn pages: the content follows the finger
/// one to one up to a third of its width, then with rubber-band damping, and
/// a release past that third (or a flick faster than 300 pt/s) turns the
/// page. With no page in that direction there is only the rubber band.
///
/// Drags that start at the leading screen edge are left to the system back
/// gesture, and a drag counts only when it is more horizontal than vertical,
/// so vertical scrolling and taps on the content keep working.
///
/// The course page uses it for lessons. TimetableScreen still has its own
/// `swipe` (weeks) and should move to this modifier in a follow-up.
struct HorizontalPager: ViewModifier {
    let canGoPrevious: Bool
    let canGoNext: Bool
    let turn: (PageStep) -> Void

    static let flickVelocity: CGFloat = 300

    @State private var offset: CGFloat = 0
    @State private var width: CGFloat = 0

    func body(content: Content) -> some View {
        content
            .offset(x: offset)
            .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width = $0 }
            .gesture(
                PagePan(
                    moved: { dx in offset = follow(dx) },
                    ended: { dx, velocity in finish(dx, velocity: velocity) }))
    }

    private func finish(_ dx: CGFloat, velocity: CGFloat) {
        let step: PageStep = dx < 0 ? .next : .previous
        let open = step == .next ? canGoNext : canGoPrevious
        // A flick counts only in the direction the page was pulled.
        let flick = dx < 0 ? -velocity : velocity
        if open, abs(dx) > width / 3 || flick > Self.flickVelocity {
            turn(step)
        }
        withAnimation(.snappy) { offset = 0 }
    }

    /// One to one up to a third of the width, then a third of the rest; only
    /// the damped pull toward an end with no page.
    private func follow(_ dx: CGFloat) -> CGFloat {
        let open = dx < 0 ? canGoNext : canGoPrevious
        guard open else { return dx / 3 }
        let limit = width / 3
        guard abs(dx) > limit else { return dx }
        let sign: CGFloat = dx < 0 ? -1 : 1
        return sign * (limit + (abs(dx) - limit) / 3)
    }
}

extension View {
    /// Turns horizontal drags into previous/next pages (`HorizontalPager`).
    func horizontalPager(canGoPrevious: Bool, canGoNext: Bool, turn: @escaping (PageStep) -> Void) -> some View {
        modifier(HorizontalPager(canGoPrevious: canGoPrevious, canGoNext: canGoNext, turn: turn))
    }
}

/// The pager's drag, as a UIKit pan. A SwiftUI `DragGesture` cannot hold off
/// the system's own horizontal pans (iOS 26's back swipe from anywhere on the
/// content, the zoom transition's interactive dismiss), which would pop the
/// page on every right swipe. This pan makes those wait for it, and declines
/// edge starts and vertical drags so they go to the back gesture and the
/// scroll view as usual.
private struct PagePan: UIGestureRecognizerRepresentable {
    /// Drags starting this close to the leading screen edge belong to the
    /// navigation back swipe.
    static let edgeInset: CGFloat = 24

    let moved: (CGFloat) -> Void
    let ended: (_ translation: CGFloat, _ velocity: CGFloat) -> Void

    func makeCoordinator(converter: CoordinateSpaceConverter) -> Coordinator { Coordinator() }

    func makeUIGestureRecognizer(context: Context) -> UIPanGestureRecognizer {
        let pan = UIPanGestureRecognizer()
        pan.delegate = context.coordinator
        return pan
    }

    func handleUIGestureRecognizerAction(_ pan: UIPanGestureRecognizer, context: Context) {
        let dx = pan.translation(in: pan.view).x
        switch pan.state {
        case .began, .changed: moved(dx)
        case .ended: ended(dx, pan.velocity(in: pan.view).x)
        case .cancelled, .failed: ended(0, 0)
        default: break
        }
    }

    final class Coordinator: NSObject, UIGestureRecognizerDelegate {
        func gestureRecognizerShouldBegin(_ recognizer: UIGestureRecognizer) -> Bool {
            guard let pan = recognizer as? UIPanGestureRecognizer else { return false }
            let translation = pan.translation(in: pan.view)
            let start = pan.location(in: nil).x - translation.x
            return start > PagePan.edgeInset && abs(translation.x) > abs(translation.y)
        }

        /// Scrolling and taps run alongside the pager.
        func gestureRecognizer(
            _ recognizer: UIGestureRecognizer,
            shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer
        ) -> Bool {
            !Self.isSystemPan(other)
        }

        /// The system's content-wide pans wait until the pager has declined.
        func gestureRecognizer(
            _ recognizer: UIGestureRecognizer,
            shouldBeRequiredToFailBy other: UIGestureRecognizer
        ) -> Bool {
            Self.isSystemPan(other)
        }

        /// A pan that is neither a scroll view's own nor the screen-edge back
        /// swipe (which the pager never competes with).
        private static func isSystemPan(_ other: UIGestureRecognizer) -> Bool {
            other is UIPanGestureRecognizer
                && !(other is UIScreenEdgePanGestureRecognizer)
                && !(other.view is UIScrollView)
        }
    }
}

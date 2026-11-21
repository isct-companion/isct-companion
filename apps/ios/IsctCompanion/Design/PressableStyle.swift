import SwiftUI

/// A press that answers at once: the card dips while the finger is down,
/// before the navigation that follows.
struct PressableStyle: ButtonStyle {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(configuration.isPressed && !reduceMotion ? 0.97 : 1)
            .opacity(configuration.isPressed ? 0.85 : 1)
            .animation(.smooth(duration: Theme.motionFast), value: configuration.isPressed)
    }
}

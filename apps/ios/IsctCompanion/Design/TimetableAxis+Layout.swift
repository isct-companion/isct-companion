import IsctCore
import SwiftUI

/// Core's timetable axis (`timetable_axis`): every period one row, lunch half
/// a row, short breaks folded into the rules. The shell only interpolates
/// between Core's stops, so desktop and iOS place times identically.
extension TimetableAxis {
    func offset(minutes: Int) -> Double {
        guard let first = stops.first, let last = stops.last else { return 0 }
        if minutes <= Int(first.minutes) { return first.offset }
        if minutes >= Int(last.minutes) { return last.offset }
        guard let index = stops.firstIndex(where: { minutes <= Int($0.minutes) }), index > 0 else {
            return last.offset
        }
        let (from, to) = (stops[index - 1], stops[index])
        let span = max(Double(to.minutes) - Double(from.minutes), 1)
        return from.offset + (to.offset - from.offset) * (Double(minutes) - Double(from.minutes)) / span
    }

    func offset(_ clock: String) -> Double { offset(minutes: Self.minutes(clock)) }

    /// Where now sits, while it falls inside the drawn day.
    func now(_ date: Date) -> Double? {
        let parts = Calendar.tokyo.dateComponents([.hour, .minute], from: date)
        let minutes = (parts.hour ?? 0) * 60 + (parts.minute ?? 0)
        guard let first = stops.first, let last = stops.last,
            minutes >= Int(first.minutes), minutes <= Int(last.minutes)
        else { return nil }
        return offset(minutes: minutes)
    }

    static func minutes(_ clock: String) -> Int {
        let parts = clock.split(separator: ":").compactMap { Int($0) }
        return parts.count == 2 ? parts[0] * 60 + parts[1] : 0
    }
}

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

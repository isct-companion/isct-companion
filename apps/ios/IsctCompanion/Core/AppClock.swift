import Foundation
import IsctCore

/// The app's one wall clock, Core's (`app_core::clock`): the system clock
/// plus Core's offset, which is zero outside the demo app. Every time the
/// app shows or compares with Core's times reads this; only system APIs that
/// run on the device's own clock (schedulers, the pasteboard, OTP codes) take
/// system dates, converted with `system(_:)`.
enum AppClock {
    static let offset = TimeInterval(clockOffsetMs()) / 1000

    static var now: Date { Date(timeIntervalSinceNow: offset) }

    /// A system clock reading, such as a `TimelineView`'s date, on this clock.
    static func at(_ system: Date) -> Date { system.addingTimeInterval(offset) }

    /// The system clock's reading when this clock reaches `date`.
    static func system(_ date: Date) -> Date { date.addingTimeInterval(-offset) }

    /// Seconds from now until `date`.
    static func until(_ date: Date) -> TimeInterval { date.timeIntervalSince(now) }
}

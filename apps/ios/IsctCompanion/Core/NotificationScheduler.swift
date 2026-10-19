import Foundation
import IsctCore
import UserNotifications

/// Registers Core's notification plan with the system. Core decides what is
/// due and when (class lead time, deadlines, settings); the shell schedules it
/// within iOS's limit of 64 pending requests.
enum NotificationScheduler {
    private static let limit = 64
    private static let horizonDays = 7
    private static let prefix = "isct."

    static func sync(_ session: Session) async {
        let center = UNUserNotificationCenter.current()
        let settings = await center.notificationSettings()
        guard settings.authorizationStatus == .authorized || settings.authorizationStatus == .provisional else { return }
        let now = Date()
        let horizon = now.addingTimeInterval(TimeInterval(horizonDays * 24 * 3600))
        guard case let .schedule(result)? = try? await session.scheduleQuery(queries: [
            .notificationPlan(now: AppModel.timestamp(now), horizon: AppModel.timestamp(horizon), locale: nil)
        ]).value(), case let .notificationPlan(plan)? = result.values.first
        else { return }
        let pending = await center.pendingNotificationRequests().map(\.identifier).filter { $0.hasPrefix(prefix) }
        center.removePendingNotificationRequests(withIdentifiers: pending)
        for item in plan.items.prefix(limit) {
            guard let at = AppModel.date(item.at), at > now else { continue }
            let content = UNMutableNotificationContent()
            content.title = item.title
            content.body = item.body
            content.sound = .default
            content.userInfo = ["deepLink": item.deepLink]
            let parts = Calendar.tokyo.dateComponents(in: .tokyo, from: at)
            let trigger = UNCalendarNotificationTrigger(
                dateMatching: DateComponents(
                    calendar: Calendar.tokyo, timeZone: .tokyo, year: parts.year, month: parts.month,
                    day: parts.day, hour: parts.hour, minute: parts.minute),
                repeats: false)
            try? await center.add(UNNotificationRequest(identifier: prefix + item.key, content: content, trigger: trigger))
        }
    }

    /// Asked the first time a reminder switch is turned on.
    static func requestPermission() async -> Bool {
        (try? await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge])) ?? false
    }
}

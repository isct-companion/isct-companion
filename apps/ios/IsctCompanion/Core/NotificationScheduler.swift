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
        let now = Date()
        let horizon = now.addingTimeInterval(TimeInterval(horizonDays * 24 * 3600))
        guard case let .schedule(result)? = try? await session.scheduleQuery(queries: [
            .notificationPlan(now: AppModel.timestamp(now), horizon: AppModel.timestamp(horizon), locale: nil)
        ]).value(), case let .notificationPlan(plan)? = result.values.first
        else { return }
        // Reminder switches sync from the other devices, so this device may
        // never have seen one turned on: it asks the first time Core has a
        // reminder for it, instead of dropping the plan unasked.
        if !plan.items.isEmpty, await center.notificationSettings().authorizationStatus == .notDetermined {
            _ = await requestPermission()
        }
        guard await allowed(center) else { return }
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

    /// Asked the first time a reminder switch is turned on, or the first time
    /// a synced switch has a reminder to deliver.
    static func requestPermission() async -> Bool {
        (try? await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge])) ?? false
    }

    static func allowed(_ center: UNUserNotificationCenter = .current()) async -> Bool {
        switch await center.notificationSettings().authorizationStatus {
        case .authorized, .provisional, .ephemeral: true
        default: false
        }
    }

    /// The desktop's Test notification: asks if this device never has, then
    /// delivers one now. False when the system does not allow notifications.
    static func sendTest() async -> Bool {
        let center = UNUserNotificationCenter.current()
        if await center.notificationSettings().authorizationStatus == .notDetermined {
            _ = await requestPermission()
        }
        guard await allowed(center) else { return false }
        let content = UNMutableNotificationContent()
        content.title = "IsctCompanion"
        content.body = t("This is a test notification.")
        content.sound = .default
        try? await center.add(UNNotificationRequest(identifier: "isct-test", content: content, trigger: nil))
        return true
    }

    /// iOS hides a notification that arrives while the app is open unless a
    /// delegate asks for it; reminders and the test show as banners then too.
    static func presentInForeground() {
        UNUserNotificationCenter.current().delegate = presenter
    }

    private static let presenter = ForegroundPresenter()
}

private final class ForegroundPresenter: NSObject, UNUserNotificationCenterDelegate {
    func userNotificationCenter(_ center: UNUserNotificationCenter,
                                willPresent notification: UNNotification) async -> UNNotificationPresentationOptions {
        [.banner, .list, .sound]
    }
}

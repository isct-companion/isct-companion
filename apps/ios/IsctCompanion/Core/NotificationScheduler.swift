import Foundation
import IsctCore
import os
import UIKit
import UserNotifications

/// Applies Core's reminder plan using the system notification center.
@MainActor
final class NotificationScheduler: NativeReminderPort {
    private static let limit = 64
    private static let prefix = "isct."
    private let center = UNUserNotificationCenter.current()
    private let log = Logger(subsystem: "app.isctcompanion", category: "reminders")
    private weak var session: Session?
    private var asking = false

    init(session: Session) {
        self.session = session
    }

    func facts() async throws -> ReminderPortFacts {
        let permitted = await Self.allowed(center)
        let requests = await center.pendingNotificationRequests()
        let pending = requests.compactMap { request -> PendingReminder? in
            guard request.identifier.hasPrefix(Self.prefix) else { return nil }
            let at = request.content.userInfo["reminderAt"] as? String
                ?? (request.trigger as? UNCalendarNotificationTrigger)?.nextTriggerDate().map { AppModel.timestamp($0) }
            return PendingReminder(key: String(request.identifier.dropFirst(Self.prefix.count)), at: at, state: .pending)
        }
        return ReminderPortFacts(permitted: permitted, availableSlots: UInt32(max(0, Self.limit - requests.count)), pending: pending)
    }

    func apply(plan: ReminderDeliveryPlan) async throws -> ReminderReceipt {
        if plan.permissionNeeded { askWhenActive() }
        var cancelled: [String] = []
        if !plan.cancel.isEmpty {
            center.removePendingNotificationRequests(withIdentifiers: plan.cancel.map { Self.prefix + $0 })
            let pending = Set(await center.pendingNotificationRequests().map(\.identifier))
            cancelled = plan.cancel.filter { !pending.contains(Self.prefix + $0) }
        }
        var failed: [String] = []
        for item in plan.deliverNow {
            if !(await add(item, identifier: "isct-now." + item.key, trigger: nil)) {
                failed.append(item.key)
            }
        }
        var registered: [String] = []
        for item in plan.register {
            guard let at = AppModel.date(item.at) else {
                log.error("Invalid reminder date from Core")
                continue
            }
            var parts = Calendar.tokyo.dateComponents([.year, .month, .day, .hour, .minute, .second], from: at)
            parts.calendar = Calendar.tokyo
            parts.timeZone = .tokyo
            let trigger = UNCalendarNotificationTrigger(dateMatching: parts, repeats: false)
            // If the request's date elapsed while applying the plan, this
            // trigger has no occurrence for the OS to register.
            guard trigger.nextTriggerDate() != nil else { continue }
            if await add(item, identifier: Self.prefix + item.key, trigger: trigger) {
                registered.append(item.key)
            }
        }
        return ReminderReceipt(registered: registered, cancelled: cancelled, failedDeliveries: failed)
    }

    private func add(_ item: NotificationItem, identifier: String, trigger: UNNotificationTrigger?) async -> Bool {
        let content = UNMutableNotificationContent()
        content.title = item.title
        content.body = item.body
        content.sound = .default
        content.userInfo = ["deepLink": item.deepLink, "reminderAt": item.at]
        do {
            try await center.add(UNNotificationRequest(identifier: identifier, content: content, trigger: trigger))
            return true
        } catch {
            log.error("Notification request failed: \(error.localizedDescription)")
            return false
        }
    }

    private func askWhenActive() {
        guard !asking, UIApplication.shared.applicationState == .active else { return }
        asking = true
        // The planner can finish this pass while the system prompt is open.
        Task {
            defer { asking = false }
            guard await center.notificationSettings().authorizationStatus == .notDetermined else { return }
            if await Self.requestPermission() {
                try? session?.refreshReminderPort()
            }
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
        content.body = t("settings.notification.thisIsATestNotification")
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

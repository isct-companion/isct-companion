import SwiftUI
import UserNotifications

/// The desktop's reminder choices (`platform/reminders.ts`) and labels
/// (`NotificationSettings.tsx`), so both ends write the same values to
/// `settings.notifications`.
enum ReminderLeads {
    static let classChoices = [5, 10, 15, 30]
    static let defaultClass = 10
    static let examChoices = [10, 30, 60, 1440, 2880]
    static let assignmentChoices = [15, 30, 60, 180, 360, 720, 1440, 2880, 4320, 10080]
    /// Core's `DEFAULT_ASSIGNMENT_LEADS`, used while the key is absent.
    static let defaultAssignment = [1440, 60]
    /// Core accepts one to eight distinct times.
    static let maxAssignment = 8

    static func label(_ minutes: Int) -> String {
        if minutes == 1440 { return t("1 day before") }
        if minutes % 1440 == 0 { return t("{{days}} days before", ["days": minutes / 1440]) }
        if minutes == 60 { return t("1 hour before") }
        if minutes % 60 == 0 { return t("{{hours}} hours before", ["hours": minutes / 60]) }
        return t("{{minutes}} minutes before", ["minutes": minutes])
    }

    static func classLead(_ settings: [String: Any]) -> Int {
        (settings["classLeadMinutes"] as? NSNumber)?.intValue ?? defaultClass
    }

    /// Exams follow the class switch and lead until set on their own (Core's
    /// `notification_plan`).
    static func exams(_ settings: [String: Any]) -> Bool {
        settings["exams"] as? Bool ?? (settings["classes"] as? Bool ?? false)
    }

    static func examLead(_ settings: [String: Any]) -> Int {
        (settings["examLeadMinutes"] as? NSNumber)?.intValue ?? classLead(settings)
    }

    static func assignmentLeads(_ settings: [String: Any]) -> [Int] {
        let stored = (settings["assignmentLeadMinutes"] as? [NSNumber])?.map(\.intValue) ?? []
        return stored.isEmpty ? defaultAssignment : stored
    }
}

/// Reminder switches and their timings; the system permission is asked on the
/// first switch-on. The switches sync between devices but the permission is
/// this device's, so the screen shows the system's answer and offers the
/// desktop's test notification.
struct NotificationsScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.openURL) private var openURL
    @State private var denied = false
    @State private var tested = false

    private var settings: [String: Any] {
        model.records.settings["notifications"] as? [String: Any] ?? [:]
    }

    var body: some View {
        Form {
            Section {
                Toggle(t("Remind me before classes"), isOn: binding("classes", current: settings["classes"] as? Bool ?? false))
                if settings["classes"] as? Bool == true {
                    Picker(t("Remind me"), selection: leadBinding("classLeadMinutes", ReminderLeads.classLead(settings))) {
                        ForEach(ReminderLeads.classChoices, id: \.self) { Text(ReminderLeads.label($0)).tag($0) }
                    }
                }
            }
            Section {
                Toggle(t("Remind me before exams"), isOn: binding("exams", current: ReminderLeads.exams(settings)))
                if ReminderLeads.exams(settings) {
                    let lead = ReminderLeads.examLead(settings)
                    Picker(t("Remind me"), selection: leadBinding("examLeadMinutes", lead)) {
                        ForEach(Array(Set(ReminderLeads.examChoices + [lead])).sorted(), id: \.self) {
                            Text(ReminderLeads.label($0)).tag($0)
                        }
                    }
                }
            }
            Section {
                Toggle(t("Remind me before assignment deadlines"), isOn: binding("assignments", current: settings["assignments"] as? Bool ?? false))
                if settings["assignments"] as? Bool == true {
                    NavigationLink(value: SettingsRoute.assignmentLeads) {
                        LabeledContent(t("Remind me")) {
                            Text(ReminderLeads.assignmentLeads(settings).sorted(by: >).map(ReminderLeads.label).joined(separator: "、"))
                                .lineLimit(2)
                        }
                    }
                }
            }
            Section {
                LabeledContent(t("Test notification")) {
                    Button(t("Send")) {
                        Task {
                            let sent = await NotificationScheduler.sendTest()
                            denied = !sent
                            tested = sent
                        }
                    }
                }
                if denied, let url = URL(string: UIApplication.openNotificationSettingsURLString) {
                    Button(t("Open Settings")) { openURL(url) }
                }
            } footer: {
                if denied {
                    Text(t("Notifications are not allowed. Allow them in the Settings app under Notifications."))
                        .foregroundStyle(Theme.danger)
                } else if tested {
                    Text(t("The system accepted the test notification. If no banner appears, check notification settings, Focus, and screen sharing."))
                }
            }
        }
        .navigationTitle(t("Notifications"))
        .navigationBarTitleDisplayMode(.inline)
        // The answer can change in the Settings app while this screen is away.
        .task(id: scenePhase) {
            if scenePhase == .active {
                let status = await UNUserNotificationCenter.current().notificationSettings().authorizationStatus
                denied = status == .denied
            }
        }
    }

    private func binding(_ key: String, current: Bool) -> Binding<Bool> {
        Binding(
            get: { current },
            set: { on in
                Task {
                    var permission = "default"
                    if on {
                        let granted = await NotificationScheduler.requestPermission()
                        denied = !granted
                        permission = granted ? "granted" : "denied"
                        guard granted else { return }
                    }
                    await model.setNotifications {
                        $0[key] = on
                        if on { $0["localPermission"] = permission }
                    }
                }
            })
    }

    private func leadBinding(_ key: String, _ current: Int) -> Binding<Int> {
        Binding(
            get: { current },
            set: { minutes in Task { await model.setNotifications { $0[key] = minutes } } })
    }
}

/// The desktop's multi-select of assignment reminder times: one to eight of
/// the offered leads, latest first.
struct AssignmentLeadsScreen: View {
    @Environment(AppModel.self) private var model

    private var selected: [Int] {
        ReminderLeads.assignmentLeads(model.records.settings["notifications"] as? [String: Any] ?? [:])
    }

    var body: some View {
        let chosen = selected
        Form {
            Section {
                ForEach(Array(Set(ReminderLeads.assignmentChoices + chosen)).sorted(by: >), id: \.self) { minutes in
                    let on = chosen.contains(minutes)
                    Button {
                        toggle(minutes, in: chosen)
                    } label: {
                        HStack {
                            Text(ReminderLeads.label(minutes)).foregroundStyle(Theme.ink)
                            Spacer()
                            if on { Image(systemName: "checkmark").foregroundStyle(Theme.accent) }
                        }
                    }
                    .disabled(on ? chosen.count == 1 : chosen.count >= ReminderLeads.maxAssignment)
                }
            } footer: {
                Text(t("Reminds you at every time you choose before the deadline (1–{{max}}).", ["max": ReminderLeads.maxAssignment]))
            }
        }
        .sensoryFeedback(.selection, trigger: chosen)
        .navigationTitle(t("Assignment reminder times"))
        .navigationBarTitleDisplayMode(.inline)
    }

    private func toggle(_ minutes: Int, in chosen: [Int]) {
        let next = chosen.contains(minutes) ? chosen.filter { $0 != minutes } : chosen + [minutes]
        guard !next.isEmpty, next.count <= ReminderLeads.maxAssignment else { return }
        Task { await model.setNotifications { $0["assignmentLeadMinutes"] = next } }
    }
}

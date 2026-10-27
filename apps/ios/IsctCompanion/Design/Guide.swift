import SwiftUI
import TipKit

// First-run guidance (slotledger `approach/first-launch`): one welcome sheet,
// then TipKit's one-time tips beside the gestures nobody can see. TipKit owns
// display frequency and persistence; tips wait until the sheet is closed.

enum Guide {
    /// Whether the welcome sheet was closed on this device.
    static let welcomeKey = "guide.welcome"
    /// Set by "Show welcome and tips again"; TipKit can only forget seen tips
    /// before it is configured, so the reset happens at the next launch.
    static let resetKey = "guide.resetTips"

    /// Tips stay hidden until the welcome sheet is gone.
    @Parameter static var welcomed: Bool = false

    static func configure() {
        if UserDefaults.standard.bool(forKey: resetKey) {
            try? Tips.resetDatastore()
            UserDefaults.standard.removeObject(forKey: resetKey)
        }
        try? Tips.configure([.displayFrequency(.immediate)])
        welcomed = UserDefaults.standard.bool(forKey: welcomeKey)
    }

    static func showAgain() {
        UserDefaults.standard.set(false, forKey: welcomeKey)
        UserDefaults.standard.set(true, forKey: resetKey)
        welcomed = false
    }
}

struct TimetableMarkTip: Tip {
    var title: Text { Text(t("Swipe to change weeks")) }
    var message: Text? { Text(t("Long-press a class to mark it as an exam or cancelled, or the Timetable tab to export the quarter.")) }
    var image: Image? { Image(systemName: "hand.draw") }
    var rules: [Rule] { #Rule(Guide.$welcomed) { $0 == true } }
}

struct LessonMarkTip: Tip {
    var title: Text { Text(t("Today's lesson")) }
    var message: Text? { Text(t("Long-press a lesson to make it the lesson of today's class.")) }
    var image: Image? { Image(systemName: "hand.tap") }
    var rules: [Rule] { #Rule(Guide.$welcomed) { $0 == true } }
}

struct AssignmentSwipeTip: Tip {
    var title: Text { Text(t("Pin or hide")) }
    var message: Text? { Text(t("Swipe right to pin an assignment, left to hide it.")) }
    var image: Image? { Image(systemName: "hand.draw") }
    var rules: [Rule] { #Rule(Guide.$welcomed) { $0 == true } }
}

struct WebSitesTip: Tip {
    var title: Text { Text(t("Switch sites")) }
    var message: Text? { Text(t("Tap the title, or long-press the Web tab, to switch school sites.")) }
    var image: Image? { Image(systemName: "globe") }
    var rules: [Rule] { #Rule(Guide.$welcomed) { $0 == true } }
}

/// The first open: the two language choices people did not know were
/// separate, then the way on to the school account.
struct WelcomeSheet: View {
    let connect: () -> Void
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Picker(t("App language"), selection: binding("locale", default: "en")) {
                        Text(verbatim: "English").tag("en")
                        Text(verbatim: "日本語").tag("ja")
                        Text(verbatim: "中文").tag("zh")
                    }
                    .pickerStyle(.segmented)
                    .listRowBackground(Theme.surface)
                } header: {
                    Text(t("App language"))
                }
                Section {
                    Picker(t("Course and syllabus language"), selection: binding("contentLanguage", default: "auto")) {
                        Text(t("Automatic")).tag("auto")
                        Text(verbatim: "English").tag("en")
                        Text(verbatim: "日本語").tag("ja")
                    }
                    .pickerStyle(.segmented)
                    .listRowBackground(Theme.surface)
                } header: {
                    Text(t("Course and syllabus language"))
                } footer: {
                    Text(t("Can differ from the app language."))
                }
                Section {
                    if model.needsAccount {
                        Button {
                            dismiss()
                            connect()
                        } label: {
                            Text(t("Connect school accounts"))
                                .font(.body.weight(.semibold))
                                .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.borderedProminent)
                        .buttonBorderShape(.capsule)
                        .controlSize(.large)
                        .listRowBackground(Color.clear)
                        .listRowInsets(EdgeInsets())
                    }
                }
            }
            .scrollContentBackground(.hidden)
            .background(Theme.bg)
            .navigationTitle(t("Welcome to IsctCompanion"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button(t(model.needsAccount ? "Later" : "Done")) { dismiss() }
                }
            }
        }
        .presentationDetents([.medium, .large])
    }

    private func binding(_ key: String, default fallback: String) -> Binding<String> {
        Binding(
            get: { model.records.settings.text(key) ?? fallback },
            set: { value in Task { await model.updateSettings { $0[key] = value } } }
        )
    }
}

extension View {
    /// Shows the welcome sheet once per device; closing it lets the tips in.
    func welcomeSheet(connect: @escaping () -> Void) -> some View {
        modifier(WelcomePresenter(connect: connect))
    }
}

private struct WelcomePresenter: ViewModifier {
    let connect: () -> Void
    @AppStorage(Guide.welcomeKey) private var welcomed = false

    func body(content: Content) -> some View {
        content.sheet(
            isPresented: Binding(get: { !welcomed }, set: { if !$0 { welcomed = true } }),
            onDismiss: { Guide.welcomed = true }
        ) {
            WelcomeSheet(connect: connect)
        }
    }
}

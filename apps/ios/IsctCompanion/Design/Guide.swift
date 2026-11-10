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
    var title: Text { Text(t("guide.swipeToChangeWeeks")) }
    var message: Text? { Text(t("guide.longPressAClassTo")) }
    var image: Image? { Image(systemName: "hand.draw") }
    var rules: [Rule] { #Rule(Guide.$welcomed) { $0 == true } }
}

struct LessonMarkTip: Tip {
    var title: Text { Text(t("guide.todaysLesson")) }
    var message: Text? { Text(t("guide.longPressALessonTo")) }
    var image: Image? { Image(systemName: "hand.tap") }
    var rules: [Rule] { #Rule(Guide.$welcomed) { $0 == true } }
}

struct AssignmentSwipeTip: Tip {
    var title: Text { Text(t("guide.pinOrHide")) }
    var message: Text? { Text(t("guide.swipeRightToPinAn")) }
    var image: Image? { Image(systemName: "hand.draw") }
    var rules: [Rule] { #Rule(Guide.$welcomed) { $0 == true } }
}

struct WebSitesTip: Tip {
    var title: Text { Text(t("common.switchSites")) }
    var message: Text? { Text(t("guide.tapTheTitleOrLong")) }
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
                    Picker(t("common.appLanguage"), selection: binding("locale", default: "en")) {
                        Text(verbatim: "English").tag("en")
                        Text(verbatim: "日本語").tag("ja")
                        Text(verbatim: "中文").tag("zh")
                    }
                    .pickerStyle(.segmented)
                    .listRowBackground(Theme.surface)
                } header: {
                    Text(t("common.appLanguage"))
                }
                Section {
                    Picker(t("guide.courseAndSyllabusLanguage"), selection: binding("contentLanguage", default: "auto")) {
                        Text(t("common.automatic")).tag("auto")
                        Text(verbatim: "English").tag("en")
                        Text(verbatim: "日本語").tag("ja")
                    }
                    .pickerStyle(.segmented)
                    .listRowBackground(Theme.surface)
                } header: {
                    Text(t("guide.courseAndSyllabusLanguage"))
                } footer: {
                    Text(t("guide.canDifferFromTheApp"))
                }
                Section {
                    if model.needsAccount {
                        Button {
                            dismiss()
                            connect()
                        } label: {
                            Text(t("common.connectSchoolAccounts"))
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
            .navigationTitle(t("common.welcomeToIsctCompanion"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button(t(model.needsAccount ? "common.later" : "common.done")) { dismiss() }
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

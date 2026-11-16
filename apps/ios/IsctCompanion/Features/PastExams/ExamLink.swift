import SafariServices
import SwiftUI

/// Where a past-exams "open on the website" link opens: the app's own
/// browser (Safari View Controller) or one of the browsers on this device.
/// The choice is this device's `examLinkTarget` and `examLinkBrowser`
/// settings: asked on the first tap, then followed; a long press on a link
/// asks again.
enum ExamBrowser: String, CaseIterable, Identifiable {
    case safari, chrome, edge, firefox, brave

    var id: String { rawValue }

    var name: String {
        switch self {
        case .safari: "Safari"
        case .chrome: "Chrome"
        case .edge: "Edge"
        case .firefox: "Firefox"
        case .brave: "Brave"
        }
    }

    /// `url` in this browser's own scheme; the schemes the app may ask about
    /// are listed under `LSApplicationQueriesSchemes`.
    func link(_ url: URL) -> URL? {
        let page = url.absoluteString
        let rest = page.replacingOccurrences(of: "https://", with: "")
        let escaped = page.addingPercentEncoding(withAllowedCharacters: .alphanumerics) ?? page
        return switch self {
        case .safari: URL(string: "x-safari-https://" + rest)
        case .chrome: URL(string: "googlechromes://" + rest)
        case .edge: URL(string: "microsoft-edge-https://" + rest)
        case .firefox: URL(string: "firefox://open-url?url=" + escaped)
        case .brave: URL(string: "brave://open-url?url=" + escaped)
        }
    }

    /// The browsers this device has, Safari first: the system knows which
    /// apps answer which scheme.
    @MainActor static var installed: [ExamBrowser] {
        let probe = URL(string: "https://sciencetokyoexam.com")!
        return allCases.filter { browser in
            browser == .safari || browser.link(probe).map(UIApplication.shared.canOpenURL) == true
        }
    }
}

extension EnvironmentValues {
    /// Opens a site link as the person chose; `repick` asks again first.
    @Entry var openExamLink: (URL, _ repick: Bool) -> Void = { _, _ in }
}

extension View {
    /// Lets the links under this view open as the person chose. Each pushed
    /// page applies it, as destinations do not inherit the root's.
    func examLinks() -> some View { modifier(ExamLinks()) }
}

private struct ExamLinks: ViewModifier {
    @Environment(AppModel.self) private var model
    @State private var asking: URL?
    @State private var inApp: URL?

    func body(content: Content) -> some View {
        content
            .environment(\.openExamLink) { url, repick in
                if !repick, let open = saved(url) { open() } else { asking = url }
            }
            .sheet(item: Binding(get: { asking.map(SafariPage.init) }, set: { asking = $0?.url })) { page in
                ExamLinkPicker(current: current) { choice in
                    asking = nil
                    Task {
                        await model.updateSettings {
                            $0["examLinkTarget"] = choice == nil ? "app" : "browser"
                            $0["examLinkBrowser"] = choice?.rawValue
                        }
                    }
                    open(page.url, in: choice)
                }
                .presentationDetents([.medium])
            }
            .sheet(item: Binding(get: { inApp.map(SafariPage.init) }, set: { inApp = $0?.url })) { page in
                SafariView(url: page.url).ignoresSafeArea()
            }
    }

    /// The saved choice, nil for the app's own browser; absent when none
    /// is saved or the browser is gone.
    private var current: ExamBrowser?? {
        let settings = model.records.settings
        switch settings.text("examLinkTarget") {
        case "app":
            return .some(nil)
        case "browser":
            guard let browser = settings.text("examLinkBrowser").flatMap(ExamBrowser.init(rawValue:)),
                  ExamBrowser.installed.contains(browser)
            else { return nil }
            return .some(browser)
        default:
            return nil
        }
    }

    private func saved(_ url: URL) -> (() -> Void)? {
        guard let choice = current else { return nil }
        return { open(url, in: choice) }
    }

    private func open(_ url: URL, in browser: ExamBrowser?) {
        guard let browser else { inApp = url; return }
        UIApplication.shared.open(browser.link(url) ?? url)
    }
}

/// The choices, as a short list from the bottom of the screen.
private struct ExamLinkPicker: View {
    let current: ExamBrowser??
    let choose: (ExamBrowser?) -> Void

    var body: some View {
        NavigationStack {
            List {
                Section {
                    row(nil, t("examShare.inAppBrowser"), symbol: "rectangle.portrait.on.rectangle.portrait")
                    ForEach(ExamBrowser.installed) { row($0, $0.name, symbol: "safari") }
                } footer: {
                    Text(t("examShare.linkChoiceHint"))
                }
            }
            .themedList()
            .navigationTitle(t("examShare.linkChoice"))
            .navigationBarTitleDisplayMode(.inline)
        }
    }

    private func row(_ browser: ExamBrowser?, _ title: String, symbol: String) -> some View {
        Button { choose(browser) } label: {
            HStack {
                Label(title, systemImage: symbol).foregroundStyle(Theme.ink)
                Spacer()
                if current == .some(browser) {
                    Image(systemName: "checkmark").foregroundStyle(Theme.accent)
                }
            }
        }
    }
}

/// A site link: a tap opens it as chosen, a long press asks again.
struct ExamLinkButton<Label: View>: View {
    @Environment(\.openExamLink) private var openExamLink
    let url: URL
    @ViewBuilder var label: Label

    var body: some View {
        // Gestures rather than a Button: a Button would also fire its tap
        // when the long press lifts.
        label
            .foregroundStyle(Theme.accent)
            .contentShape(Rectangle())
            .onTapGesture { openExamLink(url, false) }
            .onLongPressGesture { openExamLink(url, true) }
            .accessibilityAddTraits(.isButton)
            .accessibilityAction { openExamLink(url, false) }
            .accessibilityAction(named: t("examShare.linkChoice")) { openExamLink(url, true) }
    }
}

private struct SafariPage: Identifiable {
    let url: URL
    var id: URL { url }
}

private struct SafariView: UIViewControllerRepresentable {
    let url: URL

    func makeUIViewController(context: Context) -> SFSafariViewController {
        SFSafariViewController(url: url)
    }

    func updateUIViewController(_ controller: SFSafariViewController, context: Context) {}
}

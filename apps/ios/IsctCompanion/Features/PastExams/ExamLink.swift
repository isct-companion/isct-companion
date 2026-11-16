import SafariServices
import SwiftUI

/// Where a past-exams "open on the website" link opens: the app's own
/// browser (Safari View Controller, over the page) or the system browser.
/// The choice is this device's `examLinkTarget` setting: asked on the first
/// tap, then followed; a long press on a link picks again.
enum ExamLinkTarget: String {
    case app, browser
}

extension EnvironmentValues {
    /// Opens a site link as the person chose; `repick` asks again first.
    @Entry var openExamLink: (URL, _ repick: Bool) -> Void = { _, _ in }
}

extension View {
    /// Lets the links under this view open as `examLinkTarget` says. Each
    /// pushed page applies it, as destinations do not inherit the root's.
    func examLinks() -> some View { modifier(ExamLinks()) }
}

private struct ExamLinks: ViewModifier {
    @Environment(AppModel.self) private var model
    @Environment(\.openURL) private var openURL
    @State private var asking: URL?
    @State private var inApp: URL?

    func body(content: Content) -> some View {
        content
            .environment(\.openExamLink) { url, repick in
                let saved = model.records.settings.text("examLinkTarget").flatMap(ExamLinkTarget.init)
                if let saved, !repick { open(url, saved) } else { asking = url }
            }
            .confirmationDialog(
                t("examShare.openOnWeb"),
                isPresented: Binding(get: { asking != nil }, set: { if !$0 { asking = nil } }),
                titleVisibility: .visible,
                presenting: asking
            ) { url in
                Button(t("examShare.openInApp")) { choose(.app, url) }
                Button(t("examShare.openInBrowser")) { choose(.browser, url) }
                Button(t("common.cancel"), role: .cancel) {}
            } message: { _ in
                Text(t("examShare.linkChoiceHint"))
            }
            .sheet(item: Binding(get: { inApp.map(SafariPage.init) }, set: { inApp = $0?.url })) { page in
                SafariView(url: page.url).ignoresSafeArea()
            }
    }

    private func choose(_ target: ExamLinkTarget, _ url: URL) {
        Task { await model.updateSettings { $0["examLinkTarget"] = target.rawValue } }
        open(url, target)
    }

    private func open(_ url: URL, _ target: ExamLinkTarget) {
        switch target {
        case .app: inApp = url
        case .browser: openURL(url)
        }
    }
}

/// A site link: a tap opens it as chosen, a long press picks again.
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

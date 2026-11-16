import SwiftUI

/// A past-exams link to the ScienceTokyoExam website. A tap opens it in the
/// system's default browser, which the person sets in Settings; a long press
/// shares it through the system's share sheet.
struct ExamLinkButton<Label: View>: View {
    @Environment(\.openURL) private var openURL
    @State private var sharing = false
    let url: URL
    @ViewBuilder var label: Label

    var body: some View {
        // Gestures rather than a Button: a Button would also fire its tap
        // when the long press lifts.
        label
            .foregroundStyle(Theme.accent)
            .contentShape(Rectangle())
            .onTapGesture { openURL(url) }
            .onLongPressGesture { sharing = true }
            .accessibilityAddTraits(.isButton)
            .accessibilityAction { openURL(url) }
            .accessibilityAction(named: t("common.share")) { sharing = true }
            .sheet(isPresented: $sharing) {
                ShareSheet(url: url).presentationDetents([.medium, .large])
            }
    }
}

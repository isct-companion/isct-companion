import SwiftUI

/// A past-exams link to the ScienceTokyoExam website. A tap opens it in the
/// system's default browser, which the person sets in Settings; a long press
/// offers the system menu, with Share for sending it to another app.
struct ExamLinkButton<Label: View>: View {
    let url: URL
    @ViewBuilder var label: Label

    var body: some View {
        Link(destination: url) { label }
            .contextMenu {
                Link(destination: url) { SwiftUI.Label(t("examShare.openOnWeb"), systemImage: "safari") }
                ShareLink(item: url)
            } preview: {
                Text(url.absoluteString).font(.footnote).padding()
            }
    }
}

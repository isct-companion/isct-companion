import SwiftUI

struct NoticeBanner<Content: View>: View {
    let important: Bool
    @ViewBuilder let content: () -> Content

    var body: some View {
        content()
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(Theme.space7)
            .background(important ? Theme.dangerSoft : Theme.surface,
                        in: RoundedRectangle(cornerRadius: Theme.radiusSurface))
    }
}

struct ProblemBanner: View {
    let text: String
    var dismiss: (() -> Void)? = nil
    var retry: (() -> Void)? = nil

    var body: some View {
        NoticeBanner(important: true) {
            HStack(alignment: .firstTextBaseline, spacing: Theme.space5) {
                Image(systemName: "exclamationmark.circle.fill").foregroundStyle(Theme.danger)
                Text(text).foregroundStyle(Theme.ink)
                Spacer()
                if let retry { Button(t("common.retry"), action: retry).fontWeight(.semibold) }
                if let dismiss {
                    Button(action: dismiss) { Image(systemName: "xmark") }
                        .accessibilityLabel(t("common.close"))
                }
            }
            .font(.subheadline)
            .tint(Theme.accent)
        }
    }
}

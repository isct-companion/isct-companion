import IsctCore
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

struct AnnouncementBanners: View {
    let items: [AnnouncementView]

    var body: some View {
        if let first = items.first {
            banner(first)
            if items.count > 1 {
                DisclosureGroup(t("announcements.more", ["count": String(items.count - 1)])) {
                    ForEach(Array(items.dropFirst()), id: \.id) { banner($0) }
                }
            }
        }
    }

    private func banner(_ item: AnnouncementView) -> some View {
        NoticeBanner(important: item.level == .important) {
            VStack(alignment: .leading, spacing: Theme.space4) {
                Text(item.title).font(.headline)
                Text(item.body).font(.subheadline)
                if let link = item.link, let url = URL(string: link.url) {
                    Link(link.label, destination: url).font(.subheadline.weight(.semibold))
                }
            }
            .foregroundStyle(Theme.ink)
        }
    }
}

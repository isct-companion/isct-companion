import IsctCore
import SwiftUI

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

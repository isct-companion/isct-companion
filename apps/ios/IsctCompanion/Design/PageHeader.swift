import SwiftUI

/// A top-level page's title at the content's gutter, as 今日 and 時間割 draw
/// theirs. The system's large title sits at the bar's own margin, which on an
/// iPad does not line up with the list under it.
struct PageHeader<Trailing: View>: View {
    let title: String
    @ViewBuilder var trailing: Trailing

    var body: some View {
        HStack(alignment: .firstTextBaseline) {
            Text(title)
                .font(.largeTitle.bold())
                .foregroundStyle(Theme.ink)
                .lineLimit(1)
                .accessibilityAddTraits(.isHeader)
            Spacer(minLength: Theme.space4)
            trailing
        }
        .padding(.horizontal, Theme.gutter)
        .padding(.top, Theme.space4)
        .padding(.bottom, Theme.space2)
    }
}

extension PageHeader where Trailing == EmptyView {
    init(_ title: String) {
        self.init(title: title) { EmptyView() }
    }
}

extension View {
    /// A list under a `PageHeader`: its cards start at the same gutter.
    func pageListMargins() -> some View {
        contentMargins(.horizontal, Theme.gutter, for: .scrollContent)
    }
}

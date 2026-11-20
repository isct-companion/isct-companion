import IsctCore
import SwiftUI

extension CampusMapScreen {
    var searchField: some View {
        HStack(spacing: Theme.space4) {
            Image(systemName: "magnifyingglass")
                .foregroundStyle(Theme.muted)
                .accessibilityHidden(true)
            TextField(t("campusMap.search"), text: $query)
                .focused($searchFocused)
                .textInputAutocapitalization(.characters)
                .autocorrectionDisabled()
                .submitLabel(.search)
                .accessibilityIdentifier("campus-map-search")
                .disabled(!load.isReady)
            if !query.isEmpty {
                Button {
                    query = ""
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .symbolRenderingMode(.hierarchical)
                        .foregroundStyle(Theme.muted)
                        .frame(width: 28, height: CampusMapChrome.controlSide)
                }
                .accessibilityLabel(t("common.clear"))
            }
        }
        .padding(.leading, Theme.space7)
        .padding(.trailing, query.isEmpty ? Theme.space7 : Theme.space3)
        .frame(height: CampusMapChrome.controlSide)
        .floatingGlass(in: Capsule())
        .contentShape(Capsule())
        .onTapGesture { if load.isReady { searchFocused = true } }
    }

    func searchRow(_ result: CampusMapSearchResult) -> some View {
        Button {
            pick(result)
        } label: {
            VStack(alignment: .leading, spacing: Theme.space1) {
                Text(result.title)
                    .font(.body.weight(.semibold))
                    .foregroundStyle(Theme.ink)
                Text(Self.details(result))
                    .font(.footnote)
                    .foregroundStyle(Theme.muted)
                if result.noFloorPlan {
                    Text(t("campusMap.notOnFloorPlans"))
                        .font(.footnote)
                        .foregroundStyle(Theme.warn)
                }
            }
            .fixedSize(horizontal: false, vertical: true)
            .padding(.horizontal, Theme.cardPadding)
            .padding(.vertical, Theme.space4)
            .frame(maxWidth: .infinity, minHeight: minimumResultHeight, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .overlay(alignment: .bottom) { Divider().padding(.leading, Theme.cardPadding) }
    }

    /// `大岡山西9号館 · 2F · 大岡山`: the building unless it is the title, the
    /// floor, the campus.
    private static func details(_ result: CampusMapSearchResult) -> String {
        var parts: [String] = []
        if result.buildingName != result.title { parts.append(result.buildingName) }
        if let level = result.levelName { parts.append(level) }
        parts.append(campusName(result.campus))
        return parts.joined(separator: " · ")
    }

    static func campusName(_ campus: String) -> String {
        switch campus {
        case "suzukakedai": t("campusMap.suzukakedai")
        default: t("campusMap.ookayama")
        }
    }
}

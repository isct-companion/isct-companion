import IsctCore
import SwiftUI

/// Floors top floor first, as in Apple Maps. A dot marks the floor the
/// selected room is on, the entrance icon the floor the building is entered
/// on from the street. A tall building (J2・J3 has 20 floors) scrolls within
/// the space the page leaves, keeping the floor shown in view.
struct LevelPicker: View {
    let levels: [CampusMapLevel]
    let selected: String?
    let marked: String?
    let pick: (String) -> Void

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: Theme.radiusChip, style: .continuous)
        ViewThatFits(in: .vertical) {
            column
            ScrollViewReader { proxy in
                ScrollView(.vertical, showsIndicators: false) { column }
                    .onAppear { if let selected { proxy.scrollTo(selected, anchor: .center) } }
                    .onChange(of: selected) { _, id in
                        if let id { withAnimation { proxy.scrollTo(id, anchor: .center) } }
                    }
            }
        }
        .frame(width: CampusMapChrome.controlSide)
        .clipShape(shape)
        .floatingGlass(in: shape)
        .contentShape(.interaction, shape)
        .shadow(color: .black.opacity(0.12), radius: 6, y: 2)
        // Wider type would push the map's visible part to nothing.
        .dynamicTypeSize(...DynamicTypeSize.accessibility2)
    }

    private var column: some View {
        VStack(spacing: 0) {
            ForEach(levels, id: \.id) { level in
                let isSelected = level.id == selected
                Button {
                    pick(level.id)
                } label: {
                    Text(level.name)
                        .font(.footnote.weight(.semibold).monospacedDigit())
                        .foregroundStyle(isSelected ? Theme.accentInk : Theme.ink)
                        .padding(.horizontal, Theme.space3)
                        .frame(minWidth: 44, minHeight: 44)
                        .background(isSelected ? Theme.accent : .clear)
                        .overlay(alignment: .topTrailing) {
                            if level.id == marked {
                                Circle()
                                    .fill(isSelected ? Theme.accentInk : Theme.accent)
                                    .frame(width: 6, height: 6)
                                    .padding(5)
                                    .accessibilityHidden(true)
                            }
                        }
                        .overlay(alignment: .bottomLeading) {
                            if level.street {
                                Image("CampusMap/entrance")
                                    .renderingMode(.template)
                                    .resizable()
                                    .frame(width: 11, height: 11)
                                    .foregroundStyle(isSelected ? Theme.accentInk : Theme.accent)
                                    .padding(3)
                                    .accessibilityHidden(true)
                            }
                        }
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .id(level.id)
                .accessibilityLabel(t("campusMap.level", ["name": level.name]))
                .accessibilityValue(
                    [
                        level.id == marked ? t("campusMap.destinationFloor") : nil,
                        level.street ? t("campusMap.streetFloor") : nil,
                    ].compactMap { $0 }.joined(separator: ", "))
                .accessibilityAddTraits(isSelected ? .isSelected : [])
            }
        }
    }
}

import IsctCore
import SwiftUI

extension CampusMapScreen {
    func cardHeader(_ card: CampusMapCard) -> some View {
        HStack(alignment: .firstTextBaseline) {
            Text(card.title)
                .font(.title2.bold())
                .accessibilityAddTraits(.isHeader)
            Spacer()
            Button {
                send(.clear)
            } label: {
                Image(systemName: "xmark.circle.fill")
                    .symbolRenderingMode(.hierarchical)
                    .font(.title2)
                    .foregroundStyle(Theme.muted)
                    .frame(width: 44, height: 44)
            }
            .accessibilityLabel(t("campusMap.closeCard"))
        }
        .padding(.leading, Theme.cardPadding)
        .padding(.trailing, Theme.space3)
    }
}

/// Everything about the place under its title, at once: what it is, one row
/// of actions, then its entrances. The card moves the map; only "Open in
/// Maps" leaves the app.
struct CampusMapCardView: View {
    let card: CampusMapCard
    let choose: (Int) -> Void
    let select: (CampusMapPlace) -> Void
    let open: (String) -> Void
    /// The building's uploads this visit, when the card offers photos.
    let photos: FloorPhotoUploads?
    let takePhoto: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: Theme.space6) {
            Text(subtitle)
                .font(.subheadline)
                .foregroundStyle(Theme.muted)
            if card.buildingOnly {
                note(t("campusMap.buildingOnly"))
            } else if card.noFloorPlans {
                note(t("campusMap.noFloorPlans"))
            }
            actions
            if !card.entrances.isEmpty {
                entrances(card.entrances)
            }
            if !card.candidates.isEmpty {
                candidates(card.candidates)
            }
        }
        .padding(.horizontal, Theme.cardPadding)
        .padding(.bottom, Theme.cardPadding)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// `建物 · 大岡山`, `西9号館 · 2F · 大岡山`.
    private var subtitle: String {
        var parts: [String] = []
        switch card.kind {
        case .building:
            parts.append(t("campusMap.kind.building"))
        case .room:
            if card.buildingName != card.title { parts.append(card.buildingName) }
        }
        if let level = card.levelName { parts.append(level) }
        parts.append(CampusMapScreen.campusName(card.campus))
        return parts.joined(separator: " · ")
    }

    private func note(_ text: String) -> some View {
        Text(text)
            .font(.footnote)
            .foregroundStyle(Theme.muted)
            .fixedSize(horizontal: false, vertical: true)
    }

    /// The building (on a room's card), the floor photos and the maps app,
    /// sharing the row while their labels fit, stacked when they do not.
    private var actions: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: Theme.space4) { actionButtons }
            VStack(spacing: Theme.space4) { actionButtons }
        }
    }

    @ViewBuilder private var actionButtons: some View {
        if let building = card.building {
            Button {
                select(building)
            } label: {
                action(card.buildingName, systemImage: "building.2")
            }
            .buttonStyle(CapsuleButtonStyle(level: .secondary))
            .accessibilityLabel(t("campusMap.showBuilding", ["name": card.buildingName]))
        }
        if let photos {
            // Each tap opens the camera; the label tells how the photos went.
            Button(action: takePhoto) {
                if photos.sending > 0 {
                    action(t("campusMap.photos.sending"), systemImage: "arrow.up.circle")
                } else if photos.sent > 0 {
                    action(t("campusMap.photos.sent", ["count": String(photos.sent)]), systemImage: "checkmark.circle")
                } else {
                    action(t("campusMap.photos.upload"), systemImage: "camera")
                }
            }
            .buttonStyle(CapsuleButtonStyle(level: .secondary))
            .accessibilityHint(t("campusMap.photos.upload"))
        }
        Button {
            open(card.mapsUrl)
        } label: {
            action(t("campusMap.openInMaps"), systemImage: "arrow.up.forward.app")
        }
        .buttonStyle(CapsuleButtonStyle(level: .secondary))
    }

    private func action(_ title: String, systemImage: String) -> some View {
        Label(title, systemImage: systemImage)
            .font(.subheadline.weight(.semibold))
            .lineLimit(1)
            .padding(.horizontal, Theme.space6)
            .frame(maxWidth: .infinity, minHeight: 44)
    }

    /// Each entrance lights on the map, on its floor, in this same card;
    /// "Open in Maps" then pins it.
    private func entrances(_ rows: [CampusMapEntranceRow]) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(t("campusMap.entrances"))
                .font(.footnote.weight(.semibold))
                .foregroundStyle(Theme.muted)
                .padding(.bottom, Theme.space1)
            ForEach(rows, id: \.entranceId) { row in
                Button {
                    select(.entrance(id: row.entranceId))
                } label: {
                    HStack(spacing: Theme.space5) {
                        Image("CampusMap/entrance")
                            .renderingMode(.template)
                            .resizable()
                            .frame(width: 18, height: 18)
                            .foregroundStyle(Theme.accent)
                            .accessibilityHidden(true)
                        Text(row.name ?? t("campusMap.entrance"))
                            .font(row.selected ? .body.weight(.semibold) : .body)
                            .foregroundStyle(row.selected ? Theme.accent : Theme.ink)
                            .lineLimit(1)
                        Spacer(minLength: Theme.space4)
                        if let level = row.levelName {
                            Text(level)
                                .font(.subheadline.monospacedDigit())
                                .foregroundStyle(Theme.muted)
                        }
                        Image(systemName: "checkmark")
                            .font(.footnote.weight(.bold))
                            .foregroundStyle(Theme.accent)
                            .opacity(row.selected ? 1 : 0)
                            .accessibilityHidden(true)
                    }
                    .frame(minHeight: 44)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .overlay(alignment: .bottom) {
                    if row.entranceId != rows.last?.entranceId { Divider().padding(.leading, 28) }
                }
                .accessibilityLabel(
                    [row.name ?? t("campusMap.entrance"), row.levelName].compactMap { $0 }.joined(separator: ", "))
                .accessibilityAddTraits(row.selected ? .isSelected : [])
            }
        }
    }

    /// Every room an ambiguous spelling names; one is shown at a time.
    private func candidates(_ candidates: [CampusMapCandidate]) -> some View {
        VStack(alignment: .leading, spacing: Theme.space3) {
            Text(t("campusMap.chooseRoom"))
                .font(.footnote.weight(.semibold))
                .foregroundStyle(Theme.muted)
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: Theme.space4) {
                    ForEach(Array(candidates.enumerated()), id: \.offset) { index, candidate in
                        Button {
                            choose(index)
                        } label: {
                            Text(verbatim: "\(candidate.label) · \(candidate.buildingName)")
                                .font(.footnote.weight(.semibold))
                                .lineLimit(1)
                                .padding(.horizontal, Theme.space5)
                                .padding(.vertical, Theme.space4)
                                .frame(minHeight: 44)
                                .foregroundStyle(candidate.chosen ? Theme.accentInk : Theme.accent)
                                .background(candidate.chosen ? Theme.accent : Theme.accentSoft, in: Capsule())
                        }
                        .buttonStyle(PressableStyle())
                        .accessibilityAddTraits(candidate.chosen ? .isSelected : [])
                    }
                }
            }
        }
    }
}

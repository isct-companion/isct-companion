import CoreLocation
import IsctCore
import SwiftUI

/// The map opened for a room string.
struct CampusMapRequest: Identifiable {
    let id = UUID()
    let room: String
}

/// The campus map over Apple's base map, presented full screen with a close
/// button so no edge swipe competes with panning. The page is Core's: this
/// screen draws Core's scene, forwards what the person does and adds what
/// the platform has, the blue dot. There is no route: an entrance opens in
/// Maps.
struct CampusMapScreen: View {
    let request: CampusMapRequest
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openURL) private var openURL
    @State private var scene: CampusMapScene?
    @State private var opened = false
    /// Intents reach Core one after another, in the order sent.
    @State private var sending: Task<Void, Never>?
    @State private var location = CampusMapLocation()
    @State private var cardHeight: CGFloat = 0

    var body: some View {
        ZStack(alignment: .bottom) {
            FloorMapView(
                scene: scene,
                // The level picker sits at the right, the card at the bottom.
                insets: EdgeInsets(top: 56, leading: 24, bottom: cardHeight + 16, trailing: 64),
                showsPosition: location.allowed,
                onCamera: { send(.camera(camera: $0, framing: $1)) },
                // Core ignores a tap on a floor it has already left.
                onTap: { send(.tap(point: $0, levelId: $1)) },
                onEntrance: { send(.entrance(id: $0)) })
                .ignoresSafeArea()
            VStack(alignment: .leading, spacing: 8) {
                if let attribution = scene?.attribution, !attribution.isEmpty {
                    Text(attribution.joined(separator: " · "))
                        .font(.caption2)
                        .foregroundStyle(Theme.muted)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 2)
                        .floatingGlass(in: Capsule())
                }
                card
            }
            .padding(.horizontal, 12)
            .padding(.bottom, 8)
            .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { cardHeight = $0 }
        }
        .overlay(alignment: .topLeading) {
            Button {
                dismiss()
            } label: {
                Image(systemName: "xmark")
                    .font(.body.weight(.semibold))
                    .foregroundStyle(Theme.ink)
                    .frame(width: 44, height: 44)
                    .floatingGlass(in: Circle())
            }
            .accessibilityLabel(t("common.close"))
            .padding(.leading, 16)
        }
        .overlay(alignment: .trailing) {
            if let scene, !scene.levels.isEmpty {
                LevelPicker(levels: scene.levels, selected: scene.levelId, marked: scene.markedLevelId) {
                    send(.level(id: $0))
                }
                .padding(.trailing, 12)
                .padding(.bottom, cardHeight)
            }
        }
        .background(Theme.bg)
        .sensoryFeedback(.selection, trigger: scene?.levelId)
        .task {
            location.requestPermission()
            scene = await model.campusMapOpen(request.room)
            opened = true
        }
        // Also when dismissed before the open finished: Core drops it.
        .onDisappear { model.campusMapClose() }
    }

    /// Sends an intent after the ones before it and draws the answer unless
    /// a newer scene is already drawn.
    private func send(_ intent: CampusMapIntent) {
        guard let page = scene?.page else { return }
        let previous = sending
        sending = Task {
            await previous?.value
            guard let next = await model.campusMapSend(page, intent),
                next.revision > (scene?.revision ?? 0)
            else { return }
            scene = next
        }
    }

    // MARK: Card

    @ViewBuilder
    private var card: some View {
        VStack(alignment: .leading, spacing: 10) {
            if let card = scene?.card {
                Text(card.title ?? t("campusMap.entrance"))
                    .font(.headline)
                    .foregroundStyle(Theme.ink)
                    .lineLimit(2)
                    .accessibilityAddTraits(.isHeader)
                Text([Optional(card.buildingName), card.levelName].compactMap { $0 }.joined(separator: " · "))
                    .font(.subheadline)
                    .foregroundStyle(Theme.muted)
                    .lineLimit(1)
                if card.buildingOnly {
                    Text(t("campusMap.buildingOnly"))
                        .font(.footnote)
                        .foregroundStyle(Theme.muted)
                        .fixedSize(horizontal: false, vertical: true)
                }
                links(card.links)
                if !card.candidates.isEmpty {
                    candidates(card.candidates)
                }
            } else if opened {
                Text(t("campusMap.unavailable"))
                    .font(.headline)
                    .foregroundStyle(Theme.ink)
            } else {
                ProgressView()
            }
        }
        .padding(Theme.cardPadding)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Theme.surface, in: RoundedRectangle(cornerRadius: Theme.radiusPanel, style: .continuous))
        .shadow(color: .black.opacity(0.12), radius: 8, y: 2)
        .accessibilityElement(children: .contain)
    }

    /// One "Open in Maps" per link, Core's first one primary.
    private func links(_ links: [CampusMapLink]) -> some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                ForEach(Array(links.enumerated()), id: \.offset) { index, link in
                    let name = link.name ?? t("campusMap.entrance")
                    Button {
                        if let url = URL(string: link.url) { openURL(url) }
                    } label: {
                        Label(links.count == 1 ? t("campusMap.openInMaps") : name, systemImage: "map")
                            .font(.subheadline.weight(.semibold))
                            .lineLimit(1)
                            .padding(.horizontal, 14)
                            .frame(minHeight: 44)
                    }
                    .buttonStyle(CapsuleButtonStyle(level: index == 0 ? .primary : .secondary))
                    .accessibilityLabel(link.entrance
                        ? t("campusMap.openEntranceInMaps", ["name": name])
                        : t("campusMap.openInMaps"))
                }
            }
        }
    }

    /// Every room an ambiguous spelling names; one is shown at a time.
    private func candidates(_ candidates: [CampusMapCandidate]) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(t("campusMap.chooseRoom"))
                .font(.footnote.weight(.semibold))
                .foregroundStyle(Theme.muted)
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(Array(candidates.enumerated()), id: \.offset) { index, candidate in
                        Button {
                            send(.choose(index: UInt32(index)))
                        } label: {
                            Text(verbatim: "\(candidate.label) · \(candidate.buildingName)")
                                .font(.footnote.weight(.semibold))
                                .lineLimit(1)
                                .padding(.horizontal, 10)
                                .padding(.vertical, 8)
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

/// Floors top floor first, as in Apple Maps. A dot marks the floor the
/// selected room is on.
private struct LevelPicker: View {
    let levels: [CampusMapLevel]
    let selected: String?
    let marked: String?
    let pick: (String) -> Void

    var body: some View {
        VStack(spacing: 0) {
            ForEach(levels, id: \.id) { level in
                let isSelected = level.id == selected
                Button {
                    pick(level.id)
                } label: {
                    Text(level.name)
                        .font(.footnote.weight(.semibold).monospacedDigit())
                        .foregroundStyle(isSelected ? Theme.accentInk : Theme.ink)
                        .frame(width: 44, height: 40)
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
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(t("campusMap.level", ["name": level.name]))
                .accessibilityAddTraits(isSelected ? .isSelected : [])
            }
        }
        .clipShape(RoundedRectangle(cornerRadius: Theme.radiusChip, style: .continuous))
        .floatingGlass(in: RoundedRectangle(cornerRadius: Theme.radiusChip, style: .continuous))
        .shadow(color: .black.opacity(0.12), radius: 6, y: 2)
    }
}

/// While-in-use permission, asked for when the map first opens. MapKit
/// draws the blue dot itself; there is no background location.
@Observable
final class CampusMapLocation: NSObject, CLLocationManagerDelegate {
    private(set) var status: CLAuthorizationStatus
    @ObservationIgnored private let manager = CLLocationManager()

    override init() {
        status = manager.authorizationStatus
        super.init()
        manager.delegate = self
    }

    var allowed: Bool { status == .authorizedWhenInUse || status == .authorizedAlways }

    func requestPermission() {
        if status == .notDetermined { manager.requestWhenInUseAuthorization() }
    }

    func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        status = manager.authorizationStatus
    }
}

/// Opens the map for a room string. Nothing shows until Core can place the
/// string, which also means nothing shows before the map's file is stored.
struct RoomMapButton<Content: View>: View {
    let room: String
    @ViewBuilder var label: Content
    @Environment(AppModel.self) private var model
    @State private var placed = false
    @State private var request: CampusMapRequest?

    var body: some View {
        // A ZStack, not a Group: an empty Group would never run its task.
        ZStack {
            if placed {
                Button {
                    request = CampusMapRequest(room: room)
                } label: {
                    label
                }
                .buttonStyle(PressableStyle())
                .accessibilityLabel(t("campusMap.open"))
            }
        }
        // Again once the map's file lands (the reference source's update)
        // or the data changes.
        .task(id: "\(room)|\(model.contentRevision)|\(model.syncSource("references")?.lastUpdated ?? "")") {
            let known = await model.campusMapHas(room)
            guard !Task.isCancelled else { return }
            placed = known
        }
        .fullScreenCover(item: $request) { request in
            CampusMapScreen(request: request)
                .environment(model)
        }
    }
}

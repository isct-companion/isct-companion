import CoreLocation
import IsctCore
import MapKit
import SwiftUI

/// The map opened for a room string, with what Core found in it.
struct CampusMapRequest: Identifiable {
    let id = UUID()
    let room: String
    let targets: [CampusMapTarget]
}

/// The campus map over Apple's base map, presented full screen with a close
/// button so no edge swipe competes with panning. Core decides what is
/// shown: the room string's targets, which building and floor follow the
/// camera (its focus rule), what a tap hits and the Maps links; this page
/// keeps only the ids on screen and adds what the platform has, the blue
/// dot. There is no route: an entrance opens in Maps.
///
/// The card describes the selection and nothing else: its title, building,
/// floor and Maps links all come from it, so a camera move never changes
/// the card. The camera decides only which building's floor is drawn and
/// what the floor picker lists.
struct CampusMapScreen: View {
    let request: CampusMapRequest
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openURL) private var openURL
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var overview: CampusMapOverview?
    @State private var chosen = 0
    @State private var buildingId: String?
    @State private var levels: [CampusMapLevel] = []
    @State private var levelId: String?
    @State private var floor: CampusMapFloor?
    @State private var highlighted: [String] = []
    @State private var selection: Selection = .target
    @State private var framing: CampusMapFraming?
    @State private var focusTask: Task<Void, Never>?
    @State private var location = CampusMapLocation()
    @State private var cardHeight: CGFloat = 0

    /// What the card describes. A tapped room or entrance keeps the building
    /// (and floor) it was tapped on, wherever the camera goes after.
    private enum Selection {
        /// The room string's chosen target.
        case target
        /// A room tapped on the floor drawn then.
        case unit(CampusMapUnit, buildingId: String, levelId: String)
        /// An entrance tapped on the map, by id, with its building.
        case entrance(String, buildingId: String)

        var isTarget: Bool {
            if case .target = self { return true }
            return false
        }
    }

    private var target: CampusMapTarget? {
        request.targets.indices.contains(chosen) ? request.targets[chosen] : nil
    }

    private func building(_ id: String?) -> CampusMapBuilding? {
        guard let id else { return nil }
        return overview?.buildings.first { $0.id == id }
    }

    /// The building whose entrances are drawn: the one in focus, or the
    /// target's when it has no floor plans to focus on.
    private var shownBuilding: CampusMapBuilding? {
        building(buildingId ?? (target?.levels.isEmpty == true ? target?.buildingId : nil))
    }

    /// The entrance marks on the map: while a floor is shown, only the shown
    /// building's entrances on that floor; with no floor plan in focus, all
    /// of them, those without a floor too. The card's links list every
    /// entrance whatever the floor.
    private var drawnEntrances: [CampusMapEntrance] {
        let entrances = shownBuilding?.entrances ?? []
        guard let levelId else { return entrances }
        return entrances.filter { $0.levelId == levelId }
    }

    var body: some View {
        ZStack(alignment: .bottom) {
            FloorMapView(
                buildings: overview?.buildings ?? [],
                emphasizedBuildingId: buildingId ?? target?.buildingId,
                floor: floor,
                highlighted: Set(highlighted),
                entrances: drawnEntrances,
                selectedEntranceId: selectedEntranceId,
                framing: framing,
                bottomInset: cardHeight,
                showsPosition: location.allowed,
                animatesCamera: !reduceMotion,
                onCamera: cameraMoved,
                onTap: tapped,
                onEntrance: tappedEntrance)
                .ignoresSafeArea()
            VStack(alignment: .leading, spacing: 8) {
                if let attribution = overview?.attribution, !attribution.isEmpty {
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
            if !levels.isEmpty {
                LevelPicker(levels: levels, marked: markedLevel, selection: levelSelection)
                    .padding(.trailing, 12)
                    .padding(.bottom, cardHeight)
            }
        }
        .background(Theme.bg)
        .sensoryFeedback(.selection, trigger: levelId)
        .task {
            location.requestPermission()
            overview = await model.campusMapOverview()
            choose(chosen)
        }
        .task(id: levelId) {
            guard let levelId else {
                floor = nil
                return
            }
            guard let read = await model.campusMapFloor(levelId), !Task.isCancelled else { return }
            floor = read
        }
    }

    // MARK: Card

    @ViewBuilder
    private var card: some View {
        VStack(alignment: .leading, spacing: 10) {
            if let target {
                Text(title(target))
                    .font(.headline)
                    .foregroundStyle(Theme.ink)
                    .lineLimit(2)
                    .accessibilityAddTraits(.isHeader)
                Text(summary)
                    .font(.subheadline)
                    .foregroundStyle(Theme.muted)
                    .lineLimit(1)
                if selection.isTarget && target.matched == .building {
                    Text(t("campusMap.buildingOnly"))
                        .font(.footnote)
                        .foregroundStyle(Theme.muted)
                        .fixedSize(horizontal: false, vertical: true)
                }
                links
                if request.targets.count > 1 {
                    candidates
                }
            } else {
                Text(t("campusMap.unavailable"))
                    .font(.headline)
                    .foregroundStyle(Theme.ink)
            }
        }
        .padding(Theme.cardPadding)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Theme.surface, in: RoundedRectangle(cornerRadius: Theme.radiusPanel, style: .continuous))
        .shadow(color: .black.opacity(0.12), radius: 8, y: 2)
        .accessibilityElement(children: .contain)
    }

    private func title(_ target: CampusMapTarget) -> String {
        switch selection {
        case .target: target.label
        case let .unit(unit, _, _): unit.label ?? selectedBuilding?.name ?? ""
        case .entrance: selectedEntrance?.name ?? t("campusMap.entrance")
        }
    }

    /// The selection's building, never the camera's: the target's, or the
    /// one a room or entrance was tapped on.
    private var selectedBuilding: CampusMapBuilding? {
        switch selection {
        case .target: building(target?.buildingId)
        case let .unit(_, id, _), let .entrance(_, id): building(id)
        }
    }

    private var selectedEntrance: CampusMapEntrance? {
        guard case let .entrance(id, _) = selection else { return nil }
        return selectedBuilding?.entrances.first { $0.id == id }
    }

    /// Styles the tapped entrance's mark. Entrance ids are unique across
    /// buildings, so it matches a mark only while its building is drawn.
    private var selectedEntranceId: String? {
        if case let .entrance(id, _) = selection { return id }
        return nil
    }

    /// `西9号館 · 2F`: the selection's building and its floor (the target
    /// room's, the tapped room's, the tapped entrance's).
    private var summary: String {
        let name = selection.isTarget ? target?.buildingName : selectedBuilding?.name
        let level: CampusMapLevel? = switch selection {
        case .target: target?.levels.first { $0.id == target?.levelId }
        case let .unit(_, _, roomLevelId): selectedBuilding?.levels.first { $0.id == roomLevelId }
        case .entrance: selectedBuilding?.levels.first { $0.id == selectedEntrance?.levelId }
        }
        return [name, level?.name].compactMap { $0 }.joined(separator: " · ")
    }

    /// One "Open in Maps" per entrance of the selection's building, the
    /// tapped one first; the building's centre when it has none. No route is
    /// drawn here.
    @ViewBuilder
    private var links: some View {
        if let card = selectedBuilding {
            let entrances = card.entrances.sorted {
                ($0.id == selectedEntranceId ? 0 : 1) < ($1.id == selectedEntranceId ? 0 : 1)
            }
            if entrances.isEmpty {
                mapsButton(t("campusMap.openInMaps"), url: card.mapsUrl, primary: true)
            } else {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        ForEach(Array(entrances.enumerated()), id: \.element.id) { index, entrance in
                            let name = entrance.name ?? t("campusMap.entrance")
                            mapsButton(
                                entrances.count == 1 ? t("campusMap.openInMaps") : name,
                                url: entrance.mapsUrl,
                                primary: index == 0)
                                .accessibilityLabel(t("campusMap.openEntranceInMaps", ["name": name]))
                        }
                    }
                }
            }
        }
    }

    private func mapsButton(_ title: String, url: String, primary: Bool) -> some View {
        Button {
            if let link = URL(string: url) { openURL(link) }
        } label: {
            Label(title, systemImage: "map")
                .font(.subheadline.weight(.semibold))
                .lineLimit(1)
                .padding(.horizontal, 14)
                .frame(minHeight: 44)
        }
        .buttonStyle(CapsuleButtonStyle(level: primary ? .primary : .secondary))
    }

    /// Every room an ambiguous spelling names; one is shown at a time.
    private var candidates: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(t("campusMap.chooseRoom"))
                .font(.footnote.weight(.semibold))
                .foregroundStyle(Theme.muted)
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(Array(request.targets.enumerated()), id: \.offset) { index, candidate in
                        let picked = index == chosen
                        Button {
                            choose(index)
                        } label: {
                            Text(verbatim: "\(candidate.label) · \(candidate.buildingName)")
                                .font(.footnote.weight(.semibold))
                                .lineLimit(1)
                                .padding(.horizontal, 10)
                                .padding(.vertical, 8)
                                .foregroundStyle(picked ? Theme.accentInk : Theme.accent)
                                .background(picked ? Theme.accent : Theme.accentSoft, in: Capsule())
                        }
                        .buttonStyle(PressableStyle())
                        .accessibilityAddTraits(picked ? .isSelected : [])
                    }
                }
            }
        }
    }

    // MARK: Intents

    /// Opens on a target: its building and the floor Core picked, framed on
    /// the room. The camera move that follows is the app's, so it re-picks
    /// nothing.
    private func choose(_ index: Int) {
        guard request.targets.indices.contains(index) else { return }
        chosen = index
        let target = request.targets[index]
        showTarget()
        if target.levels.isEmpty {
            buildingId = nil
            levels = []
            levelId = nil
        } else {
            buildingId = target.buildingId
            levels = target.levels
            levelId = target.levelId
        }
        framing = CampusMapFraming(key: (framing?.key ?? 0) + 1, bounds: target.focus)
    }

    private var levelSelection: Binding<String?> {
        Binding(get: { levelId }, set: { levelId = $0 })
    }

    /// The card back on the chosen target, its rooms highlighted; the camera
    /// stays where it is.
    private func showTarget() {
        selection = .target
        highlighted = target?.highlightedUnitIds ?? []
    }

    /// The floor the selected room is on (the target's or a tapped one),
    /// marked while the picker lists that room's building.
    private var markedLevel: String? {
        switch selection {
        case .target:
            guard let target, target.matched == .room, target.buildingId == buildingId else { return nil }
            return target.levelId
        case let .unit(_, roomBuildingId, roomLevelId):
            return roomBuildingId == buildingId ? roomLevelId : nil
        case .entrance:
            return nil
        }
    }

    private func cameraMoved(_ report: CampusMapCameraReport) {
        focusTask?.cancel()
        let camera = CampusMapCamera(
            center: report.center, bounds: report.bounds, widthPoints: report.widthPoints,
            byUser: report.byUser, buildingId: buildingId, levelId: levelId)
        focusTask = Task {
            guard let focus = await model.campusMapFocus(camera), !Task.isCancelled else { return }
            guard focus.buildingId != buildingId || focus.levelId != levelId else { return }
            buildingId = focus.buildingId
            levels = focus.levels
            levelId = focus.levelId
        }
    }

    /// A tap on a room of the floor drawn selects it, with that floor and
    /// its building; its highlight stays while the camera is elsewhere. A
    /// tap that hits no room, or with no floor drawn, goes back to the
    /// target. Taps on marks and double taps never get here.
    private func tapped(_ point: CampusMapPoint) {
        guard let floor else {
            showTarget()
            return
        }
        Task {
            guard let unit = await model.campusMapUnit(at: point, levelId: floor.levelId) else {
                showTarget()
                return
            }
            highlighted = [unit.id]
            selection = .unit(unit, buildingId: floor.buildingId, levelId: floor.levelId)
        }
    }

    /// An entrance mark belongs to the building whose entrances are drawn.
    private func tappedEntrance(_ id: String) {
        guard let building = shownBuilding else { return }
        selection = .entrance(id, buildingId: building.id)
    }
}

/// Floors in the order of the building, top floor on top, as in Apple Maps.
/// A dot marks the floor the selected room is on.
private struct LevelPicker: View {
    let levels: [CampusMapLevel]
    let marked: String?
    @Binding var selection: String?

    init(levels: [CampusMapLevel], marked: String?, selection: Binding<String?>) {
        self.levels = levels
        self.marked = marked
        _selection = selection
    }

    var body: some View {
        VStack(spacing: 0) {
            ForEach(levels.reversed(), id: \.id) { level in
                let selected = level.id == selection
                Button {
                    selection = level.id
                } label: {
                    Text(level.name)
                        .font(.footnote.weight(.semibold).monospacedDigit())
                        .foregroundStyle(selected ? Theme.accentInk : Theme.ink)
                        .frame(width: 44, height: 40)
                        .background(selected ? Theme.accent : .clear)
                        .overlay(alignment: .topTrailing) {
                            if level.id == marked {
                                Circle()
                                    .fill(selected ? Theme.accentInk : Theme.accent)
                                    .frame(width: 6, height: 6)
                                    .padding(5)
                                    .accessibilityHidden(true)
                            }
                        }
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(t("campusMap.level", ["name": level.name]))
                .accessibilityAddTraits(selected ? .isSelected : [])
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
    @State private var targets: [CampusMapTarget] = []
    @State private var request: CampusMapRequest?

    var body: some View {
        // A ZStack, not a Group: an empty Group would never run its task.
        ZStack {
            if !targets.isEmpty {
                Button {
                    request = CampusMapRequest(room: room, targets: targets)
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
            let found = await model.campusMapTargets(room)
            guard !Task.isCancelled else { return }
            targets = found
        }
        .fullScreenCover(item: $request) { request in
            CampusMapScreen(request: request)
                .environment(model)
        }
    }
}

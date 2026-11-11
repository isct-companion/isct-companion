import CoreLocation
import IsctCore
import MapKit
import SwiftUI

/// The map opened for a room string, or with none to browse.
struct CampusMapRequest: Identifiable {
    let id = UUID()
    let room: String?
}

/// Where opening the map stands.
enum CampusMapLoad {
    case loading
    /// No map file is stored yet: it comes with the data updates.
    case missing
    case failed(String)
    case ready
}

/// What opening Core's page answered.
enum CampusMapOpening {
    case ready(CampusMapScene)
    case missing
    case failed(String)
}

/// A map read that failed, with the text to show.
struct CampusMapFailure: Error {
    let message: String
}

/// The campus map over Apple's base map, presented full screen with a close
/// button so no edge swipe competes with panning. The page is Core's: this
/// screen draws Core's scene, forwards what the person does and adds what
/// the platform has, the blue dot. There is no route: an entrance opens in
/// the maps app the settings name.
///
/// The card is part of this page, not a sheet: a sheet's heights apply only
/// at compact width (an iPad shows a centred form sheet, a landscape iPhone
/// a full-height one), and the map must know what the card covers.
struct CampusMapScreen: View {
    let request: CampusMapRequest
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openURL) private var openURL
    @State private var load = CampusMapLoad.loading
    @State private var scene: CampusMapScene?
    /// Intents reach Core one after another, in the order sent.
    @State private var sending: Task<Void, Never>?
    @State private var location = CampusMapLocation()
    @State private var follows = 0
    @State private var chrome = CampusMapChrome()
    @State private var expanded = true
    @State private var cardToggles = 0
    @State private var query = ""
    @FocusState private var searchFocused: Bool
    @State private var results: [CampusMapSearchResult] = []
    @State private var searchFailure: String?
    @State private var searching: Task<Void, Never>?

    /// The search list replaces the card while the person types.
    private var searchShown: Bool { searchFocused || !query.isEmpty }

    var body: some View {
        ZStack {
            FloorMapView(
                scene: scene,
                chrome: chrome,
                cardToggles: cardToggles,
                showsPosition: location.allowed,
                follows: follows,
                onCamera: { send(.camera(camera: $0, framing: $1)) },
                // Core ignores a tap on a floor it has already left.
                onTap: { send(.tap(point: $0, levelId: $1)) },
                onSelect: { send(.select(place: $0)) },
                onLocate: { location.requestPermission() })
                .ignoresSafeArea()
            VStack(spacing: 8) {
                topBar
                    .onGeometryChange(for: CGFloat.self) { $0.frame(in: .global).maxY } action: { chrome.top = $0 }
                if searchShown {
                    searchList
                }
                HStack(alignment: .top) {
                    Spacer(minLength: 0)
                    if !searchShown, let scene, !scene.levels.isEmpty {
                        LevelPicker(levels: scene.levels, selected: scene.levelId, marked: scene.markedLevelId) {
                            send(.level(id: $0))
                        }
                        .onGeometryChange(for: CGFloat.self) { $0.frame(in: .global).minX } action: {
                            chrome.trailing = $0
                        }
                        .onDisappear { chrome.trailing = nil }
                    }
                }
                .frame(maxHeight: .infinity, alignment: .top)
                if !searchShown {
                    bottomPanel
                        .onGeometryChange(for: CGFloat.self) { $0.frame(in: .global).minY } action: {
                            chrome.bottom = $0
                        }
                }
            }
            .padding(.horizontal, 12)
            .padding(.bottom, 8)
        }
        .background(Theme.bg)
        .sensoryFeedback(.selection, trigger: scene?.levelId)
        .onChange(of: searchShown) { _, shown in
            // The card is gone while searching: the map's visible part ends
            // at the screen's bottom.
            if shown { chrome.bottom = .greatestFiniteMagnitude }
        }
        .onChange(of: location.allowed) { _, allowed in
            if allowed && location.asked { follows += 1 }
        }
        .task { await open() }
        // Also when dismissed before the open finished: Core drops it.
        .onDisappear { model.campusMapClose() }
    }

    private func open() async {
        load = .loading
        switch await model.campusMapOpen(request.room) {
        case let .ready(opened):
            scene = opened
            load = .ready
        case .missing:
            load = .missing
        case let .failed(message):
            load = .failed(message)
        }
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

    // MARK: Search

    private var topBar: some View {
        HStack(spacing: 8) {
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
            HStack(spacing: 6) {
                Image(systemName: "magnifyingglass")
                    .foregroundStyle(Theme.muted)
                    .accessibilityHidden(true)
                TextField(t("campusMap.search"), text: $query)
                    .focused($searchFocused)
                    .textInputAutocapitalization(.characters)
                    .autocorrectionDisabled()
                    .submitLabel(.search)
                    .onSubmit { if let first = results.first { pick(first) } }
                if searchShown {
                    Button {
                        query = ""
                        searchFocused = false
                    } label: {
                        Image(systemName: "xmark.circle.fill").foregroundStyle(Theme.muted)
                    }
                    .accessibilityLabel(t("common.cancel"))
                }
            }
            .padding(.horizontal, 12)
            .frame(minHeight: 44)
            .floatingGlass(in: Capsule())
        }
        .onChange(of: query) { _, text in search(text) }
    }

    /// Asks Core for the text's places; an answer to text the person has
    /// since changed is dropped (Core echoes the text it answered). The map
    /// does not move while they type.
    private func search(_ text: String) {
        searching?.cancel()
        searchFailure = nil
        guard !text.trimmingCharacters(in: .whitespaces).isEmpty else {
            results = []
            return
        }
        searching = Task {
            try? await Task.sleep(for: .milliseconds(120))
            guard !Task.isCancelled else { return }
            let answer = await model.campusMapSearch(text)
            guard !Task.isCancelled, text == query else { return }
            switch answer {
            case let .success(found) where found.query == query:
                results = found.results
            case .success:
                break
            case let .failure(failure):
                results = []
                searchFailure = failure.message
            }
        }
    }

    private func pick(_ result: CampusMapSearchResult) {
        searchFocused = false
        query = ""
        results = []
        expanded = true
        send(.select(place: result.place))
    }

    private var searchList: some View {
        List {
            if let searchFailure {
                Text(searchFailure).foregroundStyle(Theme.danger)
            } else if results.isEmpty, !query.isEmpty {
                Text(t("campusMap.noResults")).foregroundStyle(Theme.muted)
            }
            ForEach(Array(results.enumerated()), id: \.offset) { _, result in
                Button {
                    pick(result)
                } label: {
                    VStack(alignment: .leading, spacing: 2) {
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
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .scrollDismissesKeyboard(.immediately)
        .background(Theme.surface, in: RoundedRectangle(cornerRadius: Theme.radiusPanel, style: .continuous))
        .clipShape(RoundedRectangle(cornerRadius: Theme.radiusPanel, style: .continuous))
        .frame(maxHeight: 360)
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

    // MARK: Card

    private var bottomPanel: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let attribution = scene?.attribution, !attribution.isEmpty {
                Text(attribution.joined(separator: " · "))
                    .font(.caption2)
                    .foregroundStyle(Theme.muted)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 2)
                    .floatingGlass(in: Capsule())
            }
            switch load {
            case .loading:
                panel { ProgressView().frame(maxWidth: .infinity) }
            case .missing:
                panel {
                    Text(t("campusMap.unavailable")).font(.headline).foregroundStyle(Theme.ink)
                    Text(t("campusMap.notDownloaded")).font(.subheadline).foregroundStyle(Theme.muted)
                }
            case let .failed(message):
                panel {
                    Text(t("campusMap.failed")).font(.headline).foregroundStyle(Theme.ink)
                    Text(message).font(.subheadline).foregroundStyle(Theme.muted)
                    Button(t("common.retry")) { Task { await open() } }
                        .buttonStyle(CapsuleButtonStyle(level: .secondary))
                }
            case .ready:
                if let card = scene?.card {
                    CampusMapCardView(
                        card: card,
                        expanded: expanded,
                        toggle: toggleCard,
                        close: { send(.clear) },
                        choose: { send(.choose(index: UInt32($0))) },
                        open: { if let url = URL(string: $0) { openURL(url) } })
                }
            }
        }
    }

    private func panel<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 8, content: content)
            .padding(Theme.cardPadding)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Theme.surface, in: RoundedRectangle(cornerRadius: Theme.radiusPanel, style: .continuous))
            .shadow(color: .black.opacity(0.12), radius: 8, y: 2)
    }

    private func toggleCard(_ open: Bool) {
        guard open != expanded else { return }
        cardToggles += 1
        withAnimation(.smooth(duration: Theme.motionSlow)) { expanded = open }
    }
}

/// The selection's card in two heights: folded, one line with the name and
/// floor; unfolded, everything. A tap on the handle or a vertical drag
/// switches; nothing swipes it away sideways.
private struct CampusMapCardView: View {
    let card: CampusMapCard
    let expanded: Bool
    let toggle: (Bool) -> Void
    let close: () -> Void
    let choose: (Int) -> Void
    let open: (String) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Capsule()
                .fill(Theme.line)
                .frame(width: 36, height: 5)
                .frame(maxWidth: .infinity)
                .frame(height: 12)
                .contentShape(Rectangle())
                .onTapGesture { toggle(!expanded) }
                .accessibilityElement()
                .accessibilityLabel(t(expanded ? "campusMap.foldCard" : "campusMap.unfoldCard"))
                .accessibilityAddTraits(.isButton)
                .accessibilityAction { toggle(!expanded) }
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(card.title ?? t("campusMap.entrance"))
                        .font(.headline)
                        .foregroundStyle(Theme.ink)
                        .lineLimit(expanded ? 2 : 1)
                        .accessibilityAddTraits(.isHeader)
                    Text(subtitle)
                        .font(.subheadline)
                        .foregroundStyle(Theme.muted)
                        .lineLimit(1)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
                .onTapGesture { if !expanded { toggle(true) } }
                Button(action: close) {
                    Image(systemName: "xmark")
                        .font(.footnote.weight(.bold))
                        .foregroundStyle(Theme.muted)
                        .frame(width: 30, height: 30)
                        .background(Theme.surface2, in: Circle())
                        .frame(width: 44, height: 44)
                        .contentShape(Circle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(t("campusMap.closeCard"))
            }
            if expanded {
                if card.buildingOnly {
                    note(t("campusMap.buildingOnly"))
                } else if card.noFloorPlans {
                    note(t("campusMap.noFloorPlans"))
                }
                links(card.links)
                if !card.candidates.isEmpty {
                    candidates(card.candidates)
                }
            }
        }
        .padding(.horizontal, Theme.cardPadding)
        .padding(.bottom, Theme.cardPadding)
        .padding(.top, 4)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Theme.surface, in: RoundedRectangle(cornerRadius: Theme.radiusPanel, style: .continuous))
        .shadow(color: .black.opacity(0.12), radius: 8, y: 2)
        .gesture(
            DragGesture(minimumDistance: 12)
                .onEnded { drag in
                    let height = drag.translation.height
                    guard abs(height) > abs(drag.translation.width) else { return }
                    if height > 30 { toggle(false) } else if height < -30 { toggle(true) }
                })
        .accessibilityElement(children: .contain)
    }

    /// The building unless it is the title, the floor, the campus.
    private var subtitle: String {
        var parts: [String] = []
        if card.buildingName != card.title { parts.append(card.buildingName) }
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

    /// One "Open in Maps" per link, Core's first one primary.
    private func links(_ links: [CampusMapLink]) -> some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                ForEach(Array(links.enumerated()), id: \.offset) { index, link in
                    let name = link.name ?? t("campusMap.entrance")
                    Button {
                        open(link.url)
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
                            choose(index)
                        } label: {
                            Text(verbatim: "\(candidate.label) · \(candidate.buildingName)")
                                .font(.footnote.weight(.semibold))
                                .lineLimit(1)
                                .padding(.horizontal, 10)
                                .padding(.vertical, 8)
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

/// Floors top floor first, as in Apple Maps. A dot marks the floor the
/// selected room is on. A tall building (J2・J3 has 20 floors) scrolls within
/// the space the page leaves, keeping the floor shown in view.
private struct LevelPicker: View {
    let levels: [CampusMapLevel]
    let selected: String?
    let marked: String?
    let pick: (String) -> Void

    var body: some View {
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
        .fixedSize(horizontal: true, vertical: false)
        .clipShape(RoundedRectangle(cornerRadius: Theme.radiusChip, style: .continuous))
        .floatingGlass(in: RoundedRectangle(cornerRadius: Theme.radiusChip, style: .continuous))
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
                        .padding(.horizontal, 6)
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
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .id(level.id)
                .accessibilityLabel(t("campusMap.level", ["name": level.name]))
                .accessibilityValue(level.id == marked ? t("campusMap.destinationFloor") : "")
                .accessibilityAddTraits(isSelected ? .isSelected : [])
            }
        }
    }
}

/// While-in-use permission, asked for only when the person taps the
/// position button. MapKit draws the blue dot itself; there is no
/// background location.
@Observable
final class CampusMapLocation: NSObject, CLLocationManagerDelegate {
    private(set) var status: CLAuthorizationStatus
    /// This page asked: once allowed, the map follows the person.
    private(set) var asked = false
    @ObservationIgnored private let manager = CLLocationManager()

    override init() {
        status = manager.authorizationStatus
        super.init()
        manager.delegate = self
    }

    var allowed: Bool { status == .authorizedWhenInUse || status == .authorizedAlways }

    func requestPermission() {
        guard status == .notDetermined else { return }
        asked = true
        manager.requestWhenInUseAuthorization()
    }

    func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        status = manager.authorizationStatus
    }
}

/// Opens the map for a room string, or with none to browse it. Nothing
/// shows until Core can place the string (with none, until a map file is
/// stored).
struct RoomMapButton<Content: View>: View {
    let room: String?
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
                .accessibilityLabel(t(room == nil ? "campusMap.browse" : "campusMap.open"))
            }
        }
        // Again once the map's file lands (the reference source's update)
        // or the data changes.
        .task(id: "\(room ?? "")|\(model.contentRevision)|\(model.syncSource("references")?.lastUpdated ?? "")") {
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

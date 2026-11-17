import CoreLocation
import IsctCore
import MapKit
import SwiftUI

/// The map opened for a room string, or with none to browse.
struct CampusMapRequest: Identifiable {
    let id = UUID()
    let room: String?
}

extension EnvironmentValues {
    @Entry var openCampusMap: (String?) -> Void = { _ in }
}

/// Where opening the map stands.
enum CampusMapLoad {
    case loading
    /// No map file is stored yet: it comes with the data updates.
    case missing
    case failed(String)
    case ready

    var isReady: Bool {
        if case .ready = self { true } else { false }
    }
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
/// the platform has, the blue dot. There is no route: the card's one way
/// out of the app opens the maps app the settings name.
///
/// As in Apple Maps, the search bar runs along the top and its results drop
/// under it while typing. A selection's card has one height, its content's:
/// at the bottom of a tall, narrow window, under the search bar at the
/// leading edge otherwise. The map's visible part is the region these leave,
/// measured.
struct CampusMapScreen: View {
    let request: CampusMapRequest
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openURL) private var openURL
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    @Environment(\.verticalSizeClass) private var verticalSizeClass
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var load = CampusMapLoad.loading
    @State private var scene: CampusMapScene?
    /// Intents reach Core one after another, in the order sent.
    @State private var sending: Task<Void, Never>?
    @State private var location = CampusMapLocation()
    @State private var follows = 0
    /// Floors picked in the picker: each gives a tick and a cross-fade.
    @State private var levelPicks = 0
    /// The floor just picked, lit in the picker before Core answers.
    @State private var pickedLevel: String?
    /// Rooms, buildings and entrances picked on the map: each gives a tick.
    @State private var mapPicks = 0
    /// The card's and the results' content, measured: each panel is as tall
    /// as its content up to a share of the window.
    @State private var cardHeight: CGFloat = 0
    @State private var resultsHeight: CGFloat = 0
    @ScaledMetric(relativeTo: .body) private var minimumResultHeight: CGFloat = 64
    @State private var railLeading: CGFloat?
    /// The map's visible part in window coordinates, with the request the
    /// layout that measured it answered.
    @State private var openFrame: CGRect?
    @State private var openLayout: CampusMapLayoutRequest?
    @State private var query = ""
    @FocusState private var searchFocused: Bool
    @State private var keyboardVisible = false
    @State private var pickingResult = false
    @State private var searchAnswer: CampusMapSearch?
    @State private var searchFailure: String?

    init(request: CampusMapRequest) {
        self.request = request
    }

    /// The card sits under the search bar, beside the map, in a wide or
    /// short window.
    private var wide: Bool {
        horizontalSizeClass == .regular || verticalSizeClass == .compact
    }

    private var searching: Bool { searchFocused || !query.isEmpty }
    private var showsCard: Bool { scene?.card != nil && !searching }

    private var cardSpring: Animation? {
        reduceMotion ? nil : .smooth(duration: 0.3)
    }

    private var layoutRequest: CampusMapLayoutRequest {
        CampusMapLayoutRequest(
            framing: scene?.framing?.key ?? 0, keyboardVisible: keyboardVisible,
            searching: searching, card: showsCard ? cardHeight : nil, wide: wide)
    }

    private var chrome: CampusMapChrome {
        CampusMapChrome(
            top: openFrame?.minY ?? 0,
            bottom: openFrame?.maxY ?? .greatestFiniteMagnitude,
            leading: openFrame?.minX ?? 0,
            trailing: railLeading,
            layout: openLayout)
    }

    var body: some View {
        @Bindable var location = location
        ZStack {
            FloorMapView(
                scene: scene,
                chrome: chrome,
                layoutRequest: layoutRequest,
                showsPosition: location.allowed,
                follows: follows,
                levelPicks: levelPicks,
                onCamera: { send(.camera(camera: $0)) },
                // Core ignores a tap on a floor it has already left.
                onTap: { selectOnMap(.tap(point: $0, levelId: $1, metresPerPoint: $2, roomsOnly: $3)) },
                onSelect: { selectOnMap(.select(place: $0)) },
                onLocate: { location.requestPermission() })
                .ignoresSafeArea()
            GeometryReader { proxy in
                Group {
                    if wide { wideLayout(proxy.size) } else { narrowLayout(proxy.size) }
                }
                .animation(cardSpring, value: searching)
                .animation(cardSpring, value: showsCard)
            }
        }
        .background(Theme.bg)
        // Only the person's own picks tick, not a floor the camera brings.
        .sensoryFeedback(.selection, trigger: levelPicks)
        .sensoryFeedback(.impact(weight: .light), trigger: mapPicks)
        .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillShowNotification)) { _ in
            keyboardVisible = true
        }
        // Core's framing uses the layout after keyboard dismissal.
        .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardDidHideNotification)) { _ in
            keyboardVisible = false
        }
        .onChange(of: location.allowed) { _, allowed in
            if allowed && location.asked { follows += 1 }
        }
        .alert(t("campusMap.locationAccessTitle"), isPresented: $location.settingsRequired) {
            Button(t("settings.main.openSettings")) {
                if let url = URL(string: UIApplication.openSettingsURLString) { openURL(url) }
            }
            Button(t("common.cancel"), role: .cancel) {}
        } message: {
            Text(t("campusMap.locationAccessMessage"))
        }
        .task {
            if case .loading = load { await open() }
        }
        .task(id: query) { await search(query) }
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
    private func send(_ intent: CampusMapIntent, completion: ((CampusMapScene?) -> Void)? = nil) {
        guard let page = scene?.page else { return }
        let previous = sending
        sending = Task {
            await previous?.value
            guard scene?.page == page else { return }
            let next = await model.campusMapSend(page, intent)
            guard let current = scene, current.page == page else { return }
            if let next, next.revision > current.revision {
                if (current.card == nil) != (next.card == nil) {
                    withAnimation(cardSpring) { scene = next }
                } else {
                    scene = next
                }
            }
            completion?(next)
        }
    }

    private func selectOnMap(_ intent: CampusMapIntent) {
        let before = scene?.card
        send(intent) { next in
            guard let next else { return }
            if next.card != nil, next.card != before { mapPicks += 1 }
            if searching, next.card != nil { endSearch() }
        }
    }

    // MARK: Layout

    /// Search bar, results while typing, the map's open part, the card.
    private func narrowLayout(_ size: CGSize) -> some View {
        VStack(spacing: CampusMapChrome.controlGap) {
            topBar
            if searching {
                resultsPanel(maximum: size.height * 0.5)
                    .transition(.opacity)
            }
            openRegion
            if showsCard, let card = scene?.card {
                cardPanel(card, maximum: size.height * 0.5)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            } else if !load.isReady {
                statusPanel
            }
        }
        .padding(.horizontal, CampusMapChrome.pageInset)
        .padding(.bottom, CampusMapChrome.controlGap)
    }

    /// The search bar and what it opens in a column at the leading edge.
    private func wideLayout(_ size: CGSize) -> some View {
        HStack(alignment: .top, spacing: CampusMapChrome.controlGap) {
            VStack(spacing: CampusMapChrome.controlGap) {
                topBar
                if searching {
                    resultsPanel(maximum: size.height * 0.7)
                } else if showsCard, let card = scene?.card {
                    cardPanel(card, maximum: size.height * 0.7)
                        .transition(.opacity)
                } else if !load.isReady {
                    statusPanel
                }
                Spacer(minLength: 0)
            }
            .frame(width: min(360, size.width * 0.45))
            openRegion
        }
        .padding(.horizontal, CampusMapChrome.pageInset)
        .padding(.bottom, CampusMapChrome.controlGap)
    }

    /// The part of the map the page's controls leave: the floor picker on
    /// its trailing edge, the attribution at its foot. Its frame is the
    /// visible part framings fit in.
    private var openRegion: some View {
        GeometryReader { region in
            ZStack(alignment: .topTrailing) {
                rail(available: region.size.height)
                if let attribution = scene?.attribution, !attribution.isEmpty {
                    Text(attribution.joined(separator: " · "))
                        .font(.caption2)
                        .foregroundStyle(Theme.muted)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 2)
                        .floatingGlass(in: Capsule())
                        .frame(
                            maxWidth: .infinity, maxHeight: .infinity,
                            alignment: wide ? .bottomTrailing : .bottomLeading)
                        .allowsHitTesting(false)
                }
            }
        }
        .background {
            CampusMapPanelLayoutProbe(request: layoutRequest) { request, frame in
                guard request == layoutRequest else { return }
                if openFrame != frame { openFrame = frame }
                if openLayout != request { openLayout = request }
            }
            .allowsHitTesting(false)
        }
    }

    // MARK: Search bar

    private var topBar: some View {
        HStack(spacing: CampusMapChrome.controlGap) {
            closeButton
            HStack(spacing: 8) {
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
            .padding(.leading, 14)
            .padding(.trailing, query.isEmpty ? 14 : 6)
            .frame(height: CampusMapChrome.controlSide)
            .floatingGlass(in: Capsule())
            .contentShape(Capsule())
            .onTapGesture { if load.isReady { searchFocused = true } }
            if searching {
                Button(t("common.cancel")) { endSearch() }
                    .font(.body.weight(.semibold))
                    .frame(minHeight: CampusMapChrome.controlSide)
                    .transition(.move(edge: .trailing).combined(with: .opacity))
            }
        }
    }

    private var closeButton: some View {
        Button {
            dismiss()
        } label: {
            Image(systemName: "xmark")
                .font(.body.weight(.semibold))
                .foregroundStyle(Theme.ink)
                .frame(width: CampusMapChrome.controlSide, height: CampusMapChrome.controlSide)
                .floatingGlass(in: Circle())
        }
        .accessibilityLabel(t("common.close"))
    }

    /// MapKit's compass and position button are drawn by the map itself in
    /// the space this column keeps for them.
    private func rail(available: CGFloat) -> some View {
        let pickerHeight = available - CampusMapChrome.railHeaderHeight
        return Group {
            if pickerHeight >= CampusMapChrome.controlSide {
                levels
                    .frame(height: pickerHeight, alignment: .top)
                    .padding(.top, CampusMapChrome.railHeaderHeight)
            }
        }
        .frame(width: CampusMapChrome.controlSide, height: available, alignment: .top)
        .onGeometryChange(for: CGFloat.self) { $0.frame(in: .global).minX } action: { railLeading = $0 }
    }

    @ViewBuilder
    private var levels: some View {
        if let scene, !scene.levels.isEmpty {
            LevelPicker(levels: scene.levels, selected: pickedLevel ?? scene.levelId, marked: scene.markedLevelId) { id in
                guard id != (pickedLevel ?? scene.levelId) else { return }
                pickedLevel = id
                levelPicks += 1
                send(.level(id: id)) { _ in
                    if pickedLevel == id { pickedLevel = nil }
                }
            }
        }
    }

    // MARK: Panels

    private func glass<Content: View>(_ content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: Theme.radiusPanel, style: .continuous)
        return content
            .clipShape(shape)
            .floatingGlass(in: shape)
            .contentShape(.interaction, shape)
    }

    /// One level: the place, then everything about it at once.
    private func cardPanel(_ card: CampusMapCard, maximum: CGFloat) -> some View {
        glass(
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    cardHeader(card)
                    CampusMapCardView(
                        card: card,
                        choose: { send(.choose(index: UInt32($0))) },
                        select: { selectOnMap(.select(place: $0)) },
                        open: { if let url = URL(string: $0) { openURL(url) } })
                }
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { measured in
                    if abs(measured - cardHeight) > 0.5 { cardHeight = measured }
                }
            }
            .scrollBounceBehavior(.basedOnSize)
            .frame(height: min(maximum, max(cardHeight, CampusMapChrome.controlSide))))
    }

    private func cardHeader(_ card: CampusMapCard) -> some View {
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
        .padding(.trailing, 6)
        .padding(.top, 6)
    }

    /// Loading, or the map file is missing or unreadable.
    @ViewBuilder
    private var statusPanel: some View {
        switch load {
        case .loading:
            glass(ProgressView().frame(maxWidth: .infinity, minHeight: 64))
        case .missing:
            glass(
                ContentUnavailableView(
                    t("campusMap.unavailable"), systemImage: "map",
                    description: Text(t("campusMap.notDownloaded")))
                    .frame(maxWidth: .infinity, minHeight: 180))
        case let .failed(message):
            glass(
                ContentUnavailableView {
                    Label(t("campusMap.failed"), systemImage: "exclamationmark.triangle")
                } description: {
                    Text(message)
                } actions: {
                    Button(t("common.retry")) { Task { await open() } }
                        .buttonStyle(CapsuleButtonStyle(level: .secondary))
                }
                .frame(maxWidth: .infinity, minHeight: 200))
        case .ready:
            EmptyView()
        }
    }

    // MARK: Search

    /// Asks Core for the text's places; an answer to text the person has
    /// since changed is dropped (Core echoes the text it answered). The map
    /// does not move while they type.
    private func search(_ text: String) async {
        guard !Task.isCancelled, text == query else { return }
        searchAnswer = nil
        searchFailure = nil
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        let answer = await model.campusMapSearch(text)
        guard !Task.isCancelled, searching, text == query else { return }
        switch answer {
        case let .success(found) where found.query == query:
            searchAnswer = found
        case .success:
            break
        case let .failure(failure):
            searchFailure = failure.message
        }
    }

    private func pick(_ result: CampusMapSearchResult) {
        guard scene != nil, !pickingResult else { return }
        pickingResult = true
        send(.select(place: result.place)) { next in
            pickingResult = false
            if next?.card != nil { endSearch() }
        }
    }

    private func endSearch() {
        searchFocused = false
        query = ""
        resultsHeight = 0
    }

    /// Nothing for an empty query or before Core answers the text shown.
    @ViewBuilder
    private func resultsPanel(maximum: CGFloat) -> some View {
        if let searchFailure {
            glass(
                ContentUnavailableView {
                    Label(t("campusMap.failed"), systemImage: "exclamationmark.triangle")
                } description: {
                    Text(searchFailure)
                }
                .frame(maxWidth: .infinity)
                .frame(height: min(maximum, 180)))
        } else if let answer = searchAnswer, answer.query == query {
            if answer.results.isEmpty {
                glass(
                    ContentUnavailableView.search(text: query)
                        .frame(maxWidth: .infinity)
                        .frame(height: min(maximum, 180)))
            } else {
                glass(
                    ScrollView {
                        VStack(spacing: 0) {
                            ForEach(answer.results, id: \.place) { result in
                                searchRow(result)
                            }
                        }
                        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { measured in
                            if abs(measured - resultsHeight) > 0.5 { resultsHeight = measured }
                        }
                    }
                    .scrollBounceBehavior(.basedOnSize)
                    .scrollDismissesKeyboard(.never)
                    .id(answer.query)
                    .frame(height: min(maximum, max(resultsHeight, minimumResultHeight))))
            }
        }
    }

    private func searchRow(_ result: CampusMapSearchResult) -> some View {
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
            .fixedSize(horizontal: false, vertical: true)
            .padding(.horizontal, Theme.cardPadding)
            .padding(.vertical, 8)
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

/// Everything about the place under its title, at once: what it is, one row
/// of actions, then its entrances. The card moves the map; only "Open in
/// Maps" leaves the app.
private struct CampusMapCardView: View {
    let card: CampusMapCard
    let choose: (Int) -> Void
    let select: (CampusMapPlace) -> Void
    let open: (String) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
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

    /// The building (on a room's card) and the maps app, sharing the row.
    private var actions: some View {
        HStack(spacing: 8) {
            if let building = card.building {
                Button {
                    select(building)
                } label: {
                    action(card.buildingName, systemImage: "building.2")
                }
                .buttonStyle(CapsuleButtonStyle(level: .secondary))
                .accessibilityLabel(t("campusMap.showBuilding", ["name": card.buildingName]))
            }
            Button {
                open(card.mapsUrl)
            } label: {
                action(t("campusMap.openInMaps"), systemImage: "arrow.up.forward.app")
            }
            .buttonStyle(CapsuleButtonStyle(level: .secondary))
        }
    }

    private func action(_ title: String, systemImage: String) -> some View {
        Label(title, systemImage: systemImage)
            .font(.subheadline.weight(.semibold))
            .lineLimit(1)
            .padding(.horizontal, 12)
            .frame(maxWidth: .infinity, minHeight: 44)
    }

    /// Each entrance lights on the map, on its floor, in this same card;
    /// "Open in Maps" then pins it.
    private func entrances(_ rows: [CampusMapEntranceRow]) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(t("campusMap.entrances"))
                .font(.footnote.weight(.semibold))
                .foregroundStyle(Theme.muted)
                .padding(.bottom, 2)
            ForEach(rows, id: \.entranceId) { row in
                Button {
                    select(.entrance(id: row.entranceId))
                } label: {
                    HStack(spacing: 10) {
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
                        Spacer(minLength: 8)
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
/// selected room is on, the entrance icon the floor the building is entered
/// on from the street. A tall building (J2・J3 has 20 floors) scrolls within
/// the space the page leaves, keeping the floor shown in view.
private struct LevelPicker: View {
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

/// While-in-use permission, asked for only when the person taps the
/// position button. MapKit draws the blue dot itself; there is no
/// background location.
@Observable
final class CampusMapLocation: NSObject, CLLocationManagerDelegate {
    private(set) var status: CLAuthorizationStatus
    /// This page asked: once allowed, the map follows the person.
    private(set) var asked = false
    var settingsRequired = false
    @ObservationIgnored private let manager = CLLocationManager()

    override init() {
        status = manager.authorizationStatus
        super.init()
        manager.delegate = self
    }

    var allowed: Bool { status == .authorizedWhenInUse || status == .authorizedAlways }

    func requestPermission() {
        switch status {
        case .notDetermined:
            asked = true
            manager.requestWhenInUseAuthorization()
        case .denied, .restricted:
            settingsRequired = true
        case .authorizedWhenInUse, .authorizedAlways:
            break
        @unknown default:
            settingsRequired = true
        }
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
    @Environment(\.openCampusMap) private var openCampusMap
    @State private var placed = false

    var body: some View {
        // A ZStack, not a Group: an empty Group would never run its task.
        ZStack {
            if placed {
                Button {
                    openCampusMap(room)
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
    }
}

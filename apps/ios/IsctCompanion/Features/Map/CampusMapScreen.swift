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
/// the platform has, the blue dot. There is no route: an entrance opens in
/// the maps app the settings name.
///
/// As in Apple Maps, search, its results and the place card share one panel.
/// The panel sits at the bottom in a tall, narrow window and at the leading
/// edge otherwise. The map's visible part comes from the panel's measured frame.
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
    @State private var height: CampusMapPanelHeight
    @GestureState(resetTransaction: Transaction(animation: .smooth(duration: Theme.motionSlow)))
    private var panelDrag: CGFloat = 0
    /// The collapsed panel fits the panel header, including at larger text sizes.
    @State private var headerHeight: CGFloat = 64
    @ScaledMetric(relativeTo: .subheadline) private var cardDetailsHeight: CGFloat = 96
    @State private var closeBottom: CGFloat = 0
    @State private var railLeading: CGFloat?
    /// The panel's frame in window coordinates, with the request the layout
    /// that measured it answered.
    @State private var panelFrame: CGRect?
    @State private var panelLayout: CampusMapLayoutRequest?
    @State private var query = ""
    @FocusState private var searchFocused: Bool
    @State private var keyboardVisible = false
    @State private var pickingResult = false
    @State private var searchAnswer: CampusMapSearch?
    @State private var searchFailure: String?

    init(request: CampusMapRequest) {
        self.request = request
        _height = State(initialValue: request.room == nil ? .small : .medium)
    }

    /// The leading panel leaves room beside it in a wide or short window.
    private var wide: Bool {
        horizontalSizeClass == .regular || verticalSizeClass == .compact
    }

    private var searching: Bool { searchFocused || !query.isEmpty }

    private var layoutRequest: CampusMapLayoutRequest {
        CampusMapLayoutRequest(
            framing: scene?.framing?.key ?? 0, keyboardVisible: keyboardVisible,
            height: height, headerHeight: headerHeight, dragging: panelDrag != 0, wide: wide)
    }

    private var chrome: CampusMapChrome {
        CampusMapChrome(
            top: wide ? 0 : closeBottom,
            bottom: wide ? .greatestFiniteMagnitude : panelFrame?.minY ?? .greatestFiniteMagnitude,
            leading: wide ? panelFrame?.maxX ?? 0 : 0,
            trailing: railLeading,
            layout: panelLayout)
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
                onCamera: { send(.camera(camera: $0, framing: $1)) },
                // Core ignores a tap on a floor it has already left.
                onTap: { selectOnMap(.tap(point: $0, levelId: $1)) },
                onSelect: { selectOnMap(.select(place: $0)) },
                onLocate: { location.requestPermission() })
                .ignoresSafeArea()
            GeometryReader { proxy in
                ZStack(alignment: .top) {
                    mapControls(in: proxy)
                    if !wide {
                        bottomPanel(available: proxy.size.height)
                            .padding(.horizontal, CampusMapChrome.pageInset)
                            .padding(.bottom, CampusMapChrome.controlGap)
                            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottom)
                    }
                }
            }
        }
        .background(Theme.bg)
        .sensoryFeedback(.selection, trigger: scene?.levelId)
        .onChange(of: searching) { _, active in
            if active {
                height = .search
            } else if height == .search {
                height = .small
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillShowNotification)) { _ in
            keyboardVisible = true
        }
        // Core's framing uses the panel's layout after keyboard dismissal.
        .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardDidHideNotification)) { _ in
            keyboardVisible = false
        }
        .onChange(of: scene?.card != nil) { _, selected in
            guard !searching else { return }
            if !selected {
                height = .small
            } else if height == .small {
                height = .medium
            }
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
        if !load.isReady, height == .small { height = .medium }
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
            if let next, next.revision > current.revision { scene = next }
            completion?(next)
        }
    }

    private func selectOnMap(_ intent: CampusMapIntent) {
        send(intent) { next in
            if searching, next?.card != nil { endSearch(showCard: true) }
        }
    }

    // MARK: Map controls

    /// The close button, the floor picker and MapKit's buttons over the map.
    /// The trailing rail stands above the bottom panel or beside the leading panel.
    private func mapControls(in proxy: GeometryProxy) -> some View {
        let frame = proxy.frame(in: .global)
        let panelTop = wide ? frame.maxY : min(frame.maxY, panelFrame?.minY ?? frame.maxY)
        let railHeight = max(panelTop - frame.minY - CampusMapChrome.controlGap, 0)
        return ZStack(alignment: .topLeading) {
            if wide {
                VStack(alignment: .leading, spacing: CampusMapChrome.controlGap) {
                    closeButton
                    floatingPanel(height: frame.height - CampusMapChrome.controlSide - 2 * CampusMapChrome.controlGap)
                        .frame(width: min(360, frame.width * 0.45))
                    Spacer(minLength: 0)
                }
                .padding(.bottom, CampusMapChrome.controlGap)
            } else {
                closeButton
                    .onGeometryChange(for: CGFloat.self) { $0.frame(in: .global).maxY } action: { closeBottom = $0 }
            }
            rail
                .frame(height: railHeight, alignment: wide ? .top : .bottom)
                .frame(maxWidth: .infinity, alignment: .trailing)
            if let attribution = scene?.attribution, !attribution.isEmpty {
                Text(attribution.joined(separator: " · "))
                    .font(.caption2)
                    .foregroundStyle(Theme.muted)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 2)
                    .floatingGlass(in: Capsule())
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: wide ? .bottomTrailing : .bottomLeading)
                    .frame(height: railHeight)
                    .allowsHitTesting(false)
            }
        }
        .padding(.horizontal, CampusMapChrome.pageInset)
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
    private var rail: some View {
        VStack(spacing: 0) {
            if !wide { levels }
            Color.clear
                .frame(height: CampusMapChrome.railHeaderHeight)
                .allowsHitTesting(false)
            if wide { levels }
        }
        .frame(width: CampusMapChrome.controlSide)
        .onGeometryChange(for: CGFloat.self) { $0.frame(in: .global).minX } action: { railLeading = $0 }
    }

    @ViewBuilder
    private var levels: some View {
        if let scene, !scene.levels.isEmpty {
            LevelPicker(levels: scene.levels, selected: scene.levelId, marked: scene.markedLevelId) {
                send(.level(id: $0))
            }
        }
    }

    // MARK: Panel

    // UISheetPresentationController blocks touches outside the sheet while
    // the keyboard is up, even at an undimmed detent. Keep the panel in the map.
    private func bottomPanel(available: CGFloat) -> some View {
        let cardVisible = scene?.card != nil && !searching
        let expandedCardHeight = min(available * 0.5, headerHeight + cardDetailsHeight)
        let maximum = max(headerHeight, cardVisible ? expandedCardHeight : available - CampusMapChrome.controlGap)
        let resting: CGFloat = switch height {
        case .small: headerHeight
        case .search: max(headerHeight, min(headerHeight + 156, available * 0.6))
        case .medium: cardVisible ? expandedCardHeight : available * 0.5
        }
        return framedPanel(height: min(maximum, max(headerHeight, resting - panelDrag)))
    }

    private func floatingPanel(height column: CGFloat) -> some View {
        framedPanel(height: height == .small ? min(headerHeight, column) : max(column, headerHeight))
            .environment(\.horizontalSizeClass, .compact)
    }

    private func framedPanel(height panelHeight: CGFloat) -> some View {
        let shape = RoundedRectangle(cornerRadius: Theme.radiusPanel, style: .continuous)
        return panel
            .frame(height: panelHeight)
            .clipShape(shape)
            .floatingGlass(in: shape)
            .contentShape(shape)
            .animation(reduceMotion ? nil : .smooth(duration: Theme.motionSlow), value: height)
            .transaction { if reduceMotion { $0.animation = nil } }
            .background {
                CampusMapPanelLayoutProbe(request: layoutRequest) { request, frame in
                    guard request == layoutRequest else { return }
                    if panelFrame != frame { panelFrame = frame }
                    if panelLayout != request { panelLayout = request }
                }
                .allowsHitTesting(false)
            }
    }

    /// Both containers keep the same panel while the keyboard leaves and
    /// Core's card takes the search results' place.
    private var panel: some View {
        VStack(spacing: 0) {
            Group {
                if let card = scene?.card, !searching {
                    VStack(spacing: 0) {
                        if !wide { cardGrabber }
                        cardHeader(card)
                    }
                } else {
                    searchHeader
                }
            }
            .padding(.horizontal, Theme.cardPadding)
            .padding(.vertical, 10)
            .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { measured in
                if measured > 0, abs(measured - headerHeight) > 0.5 { headerHeight = measured }
            }

            root
        }
    }

    private var cardGrabber: some View {
        Button {
            height = height == .small ? .medium : .small
        } label: {
            Capsule()
                .fill(Theme.muted.opacity(0.55))
                .frame(width: 36, height: 5)
                .frame(maxWidth: .infinity, minHeight: 20)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .gesture(
            DragGesture(minimumDistance: 10, coordinateSpace: .global)
                .updating($panelDrag) { drag, translation, transaction in
                    guard abs(drag.translation.height) > abs(drag.translation.width) else { return }
                    transaction.animation = nil
                    translation = drag.translation.height
                }
                .onEnded { drag in
                    guard abs(drag.translation.height) > abs(drag.translation.width) else { return }
                    if drag.predictedEndTranslation.height < -30 { resizeCard(up: true) }
                    if drag.predictedEndTranslation.height > 30 { resizeCard(up: false) }
                })
        .accessibilityLabel(t(height == .small ? "campusMap.unfoldCard" : "campusMap.foldCard"))
        .accessibilityAdjustableAction { direction in
            switch direction {
            case .increment: resizeCard(up: true)
            case .decrement: resizeCard(up: false)
            @unknown default: break
            }
        }
    }

    private func resizeCard(up: Bool) {
        height = up ? .medium : .small
    }

    private var searchHeader: some View {
        HStack(spacing: 8) {
            HStack(spacing: 8) {
                Image(systemName: "magnifyingglass")
                    .foregroundStyle(Theme.muted)
                    .accessibilityHidden(true)
                TextField(t("campusMap.search"), text: $query)
                    .focused($searchFocused)
                    .textInputAutocapitalization(.characters)
                    .autocorrectionDisabled()
                    .accessibilityIdentifier("campus-map-search")
                    .disabled(!load.isReady)
            }
            .padding(.horizontal, 12)
            .frame(minHeight: 44)
            .background(Theme.surface2, in: RoundedRectangle(cornerRadius: 12))

            if searching {
                Button(t("common.cancel")) { endSearch(showCard: scene?.card != nil) }
                    .frame(minHeight: 44)
            }
        }
    }

    private func cardHeader(_ card: CampusMapCard) -> some View {
        HStack {
            Text(card.title ?? t("campusMap.entrance"))
                .font(.title2.bold())
                .accessibilityAddTraits(.isHeader)
            Spacer()
            Button(action: closeCard) {
                Image(systemName: "xmark.circle.fill")
                    .symbolRenderingMode(.hierarchical)
                    .font(.title2)
                    .foregroundStyle(Theme.muted)
                    .frame(width: 44, height: 44)
            }
            .accessibilityLabel(t("campusMap.closeCard"))
        }
    }

    private var root: some View {
        Group {
            switch load {
            case .loading:
                ProgressView()
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
                    .padding(.top, Theme.cardPadding)
            case .missing:
                ContentUnavailableView(
                    t("campusMap.unavailable"), systemImage: "map",
                    description: Text(t("campusMap.notDownloaded")))
            case let .failed(message):
                ContentUnavailableView {
                    Label(t("campusMap.failed"), systemImage: "exclamationmark.triangle")
                } description: {
                    Text(message)
                } actions: {
                    Button(t("common.retry")) { Task { await open() } }
                        .buttonStyle(CapsuleButtonStyle(level: .secondary))
                }
            case .ready:
                if searching {
                    results
                } else if let card = scene?.card, height != .small {
                    CampusMapCardView(
                        card: card,
                        choose: { send(.choose(index: UInt32($0))) },
                        open: { if let url = URL(string: $0) { openURL(url) } })
                } else {
                    Color.clear
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
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
            if next?.card != nil { endSearch(showCard: true) }
        }
    }

    private func endSearch(showCard: Bool) {
        height = showCard ? .medium : .small
        searchFocused = false
        query = ""
    }

    /// Nothing for an empty query or before Core answers the text shown.
    @ViewBuilder
    private var results: some View {
        if let searchFailure {
            ContentUnavailableView {
                Label(t("campusMap.failed"), systemImage: "exclamationmark.triangle")
            } description: {
                Text(searchFailure)
            }
        } else if let answer = searchAnswer, answer.query == query {
            if answer.results.isEmpty {
                ContentUnavailableView.search(text: query)
            } else {
                List(Array(answer.results.enumerated()), id: \.offset) { _, result in
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
                    .listRowBackground(Color.clear)
                }
                .listStyle(.plain)
                .scrollContentBackground(.hidden)
                .scrollDismissesKeyboard(.never)
            }
        } else {
            Color.clear
        }
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

    private func closeCard() {
        send(.clear)
        height = .small
    }
}

/// Details below the place header, when the panel is expanded.
private struct CampusMapCardView: View {
    let card: CampusMapCard
    let choose: (Int) -> Void
    let open: (String) -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 10) {
                Text(subtitle)
                    .font(.subheadline)
                    .foregroundStyle(Theme.muted)
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
            .padding(.horizontal, Theme.cardPadding)
            .padding(.bottom, Theme.cardPadding)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
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
        .frame(width: CampusMapChrome.controlSide)
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

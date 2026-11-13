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
    @State private var cardExpanded = true
    @State private var cardExpansion: CGFloat = 1
    @State private var cardDragStart: CGFloat?
    @GestureState private var draggingCard = false
    @State private var cardHeaderHeight: CGFloat = 64
    @State private var searchHeaderHeight: CGFloat = 64
    @ScaledMetric(relativeTo: .subheadline) private var cardDetailsHeight: CGFloat = 96
    @ScaledMetric(relativeTo: .body) private var minimumResultHeight: CGFloat = 64
    @State private var firstResultHeight: CGFloat = 0
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
    }

    /// The leading panel leaves room beside it in a wide or short window.
    private var wide: Bool {
        horizontalSizeClass == .regular || verticalSizeClass == .compact
    }

    private var searching: Bool { searchFocused || !query.isEmpty }
    private var showsCard: Bool { scene?.card != nil && !searching }
    private var headerHeight: CGFloat { showsCard ? cardHeaderHeight : searchHeaderHeight }

    private var height: CampusMapPanelHeight {
        if searching { return .search }
        if showsCard { return cardExpanded || wide ? .medium : .small }
        switch load {
        case .loading, .ready: return .small
        case .missing, .failed: return .medium
        }
    }

    private var cardSpring: Animation? {
        reduceMotion ? nil : .smooth(duration: 0.32)
    }

    private var layoutRequest: CampusMapLayoutRequest {
        CampusMapLayoutRequest(
            framing: scene?.framing?.key ?? 0, keyboardVisible: keyboardVisible,
            height: height, headerHeight: headerHeight, dragging: draggingCard, wide: wide)
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
                VStack(spacing: CampusMapChrome.controlGap) {
                    GeometryReader { controls in
                        mapControls(available: controls.size)
                    }
                    if !wide {
                        bottomPanel(available: proxy.size.height)
                            .padding(.horizontal, CampusMapChrome.pageInset)
                            .padding(.bottom, CampusMapChrome.controlGap)
                            .layoutPriority(1)
                    }
                }
                .animation(cardSpring, value: searching)
            }
        }
        .background(Theme.bg)
        .sensoryFeedback(.selection, trigger: scene?.levelId)
        .onChange(of: draggingCard) { _, dragging in
            if !dragging, cardDragStart != nil { settleCard(expanded: cardExpanded) }
        }
        .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillShowNotification)) { _ in
            keyboardVisible = true
        }
        // Core's framing uses the panel's layout after keyboard dismissal.
        .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardDidHideNotification)) { _ in
            keyboardVisible = false
        }
        .onChange(of: scene?.card != nil) { _, hasCard in
            if !hasCard { resetCardExpansion() }
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
                if case .clear = intent, current.card != nil, next.card == nil {
                    withAnimation(cardSpring) { scene = next }
                } else {
                    scene = next
                }
            }
            completion?(next)
        }
    }

    private func selectOnMap(_ intent: CampusMapIntent) {
        send(intent) { next in
            if searching, next?.card != nil { endSearch(showCard: true) }
        }
    }

    // MARK: Map controls

    /// In a narrow window this region gets the space left by the bottom panel.
    private func mapControls(available: CGSize) -> some View {
        ZStack(alignment: .topLeading) {
            if wide {
                VStack(alignment: .leading, spacing: CampusMapChrome.controlGap) {
                    closeButton
                    floatingPanel(height: available.height - CampusMapChrome.controlSide - 2 * CampusMapChrome.controlGap)
                        .frame(width: min(360, available.width * 0.45))
                    Spacer(minLength: 0)
                }
                .padding(.bottom, CampusMapChrome.controlGap)
            } else {
                closeButton
                    .onGeometryChange(for: CGFloat.self) { $0.frame(in: .global).maxY } action: { closeBottom = $0 }
            }
            rail(available: available.height)
                .frame(maxWidth: .infinity, alignment: .trailing)
            if let attribution = scene?.attribution, !attribution.isEmpty {
                Text(attribution.joined(separator: " · "))
                    .font(.caption2)
                    .foregroundStyle(Theme.muted)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 2)
                    .floatingGlass(in: Capsule())
                    .frame(maxWidth: .infinity, alignment: wide ? .trailing : .leading)
                    .frame(height: max(0, available.height - (wide ? CampusMapChrome.controlGap : 0)), alignment: .bottom)
                    .clipped()
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
            LevelPicker(levels: scene.levels, selected: scene.levelId, marked: scene.markedLevelId) {
                send(.level(id: $0))
            }
        }
    }

    // MARK: Panel

    // UISheetPresentationController blocks touches outside the sheet while
    // the keyboard is up, even at an undimmed detent. Keep the panel in the map.
    private func bottomPanel(available: CGFloat) -> some View {
        let maximum = max(0, available - 2 * CampusMapChrome.controlGap)
        let detailHeight = max(0, min(cardDetailsHeight, maximum * 0.5 - headerHeight))
        let panelHeight: CGFloat
        if showsCard {
            panelHeight = headerHeight + detailHeight * cardExpansion
        } else if searching {
            let row = max(minimumResultHeight, firstResultHeight)
            let oneResult = headerHeight + row
            let preferred = headerHeight + 3 * row
            // Use more of a short viewport when the first complete row needs it.
            panelHeight = max(oneResult, min(preferred, maximum * 0.62))
        } else {
            panelHeight = height == .small ? headerHeight : max(headerHeight, maximum * 0.5)
        }
        return framedPanel(
            height: min(maximum, panelHeight), detailHeight: showsCard ? detailHeight : nil)
    }

    private func floatingPanel(height column: CGFloat) -> some View {
        framedPanel(height: height == .small ? min(headerHeight, column) : max(column, headerHeight))
            .environment(\.horizontalSizeClass, .compact)
    }

    private func framedPanel(height panelHeight: CGFloat, detailHeight: CGFloat? = nil) -> some View {
        let shape = RoundedRectangle(cornerRadius: Theme.radiusPanel, style: .continuous)
        return panel(detailHeight: detailHeight)
            .frame(height: panelHeight, alignment: .top)
            .clipShape(shape)
            .floatingGlass(in: shape)
            .contentShape(.interaction, shape)
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
    private func panel(detailHeight: CGFloat?) -> some View {
        VStack(spacing: 0) {
            ZStack(alignment: .top) {
                // Keep the search header measured for the transition from a place.
                searchHeader
                    .modifier(CampusMapPanelHeader(height: $searchHeaderHeight))
                    .opacity(showsCard ? 0 : 1)
                    .allowsHitTesting(!showsCard)
                    .accessibilityHidden(showsCard)
                if let card = scene?.card, showsCard {
                    VStack(spacing: 0) {
                        if let detailHeight { cardGrabber(travel: detailHeight) }
                        cardHeader(card)
                    }
                    .modifier(CampusMapPanelHeader(height: $cardHeaderHeight))
                    .transition(.opacity)
                }
            }
            .frame(height: headerHeight, alignment: .top)

            panelBody(detailHeight: detailHeight)
        }
    }

    private func cardGrabber(travel: CGFloat) -> some View {
        Button {
            settleCard(expanded: !cardExpanded)
        } label: {
            Capsule()
                .fill(Theme.muted.opacity(0.55))
                .frame(width: 36, height: 5)
                .frame(maxWidth: .infinity, minHeight: 20)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(travel <= 0)
        .gesture(
            DragGesture(minimumDistance: 10, coordinateSpace: .global)
                .updating($draggingCard) { drag, active, _ in
                    if travel > 0, abs(drag.translation.height) > abs(drag.translation.width) { active = true }
                }
                .onChanged { drag in
                    guard travel > 0 else { return }
                    if cardDragStart == nil {
                        guard abs(drag.translation.height) > abs(drag.translation.width) else { return }
                        let current = panelFrame.map { ($0.height - headerHeight) / travel } ?? cardExpansion
                        cardDragStart = min(1.1, max(-0.1, current))
                    }
                    guard let start = cardDragStart else { return }
                    let position = start - drag.translation.height / travel
                    let anchor = min(1, max(0, position))
                    let excess = position - anchor
                    // Keep the two resting heights while softening the travel at each edge.
                    cardExpansion = anchor + excess / (1 + 10 * abs(excess))
                }
                .onEnded { drag in
                    guard let start = cardDragStart else { return }
                    let projected = start - drag.predictedEndTranslation.height / travel
                    settleCard(expanded: projected >= 0.5)
                })
        .accessibilityLabel(t(cardExpanded ? "campusMap.foldCard" : "campusMap.unfoldCard"))
        .accessibilityAdjustableAction { direction in
            switch direction {
            case .increment: settleCard(expanded: true)
            case .decrement: settleCard(expanded: false)
            @unknown default: break
            }
        }
    }

    private func settleCard(expanded: Bool) {
        cardDragStart = nil
        withAnimation(cardSpring) {
            cardExpanded = expanded
            cardExpansion = expanded ? 1 : 0
        }
    }

    private func resetCardExpansion() {
        cardDragStart = nil
        cardExpanded = true
        cardExpansion = 1
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
    }

    private func panelBody(detailHeight: CGFloat?) -> some View {
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
                } else if let card = scene?.card {
                    if let detailHeight {
                        let interactive = cardExpanded && !draggingCard && detailHeight > 0
                        // The overlay keeps its scroll size while the visible area shrinks.
                        Color.clear.overlay(alignment: .top) {
                            cardDetails(card)
                                .frame(height: detailHeight)
                        }
                        .clipped()
                        .contentShape(.interaction, Rectangle())
                        .allowsHitTesting(interactive)
                        .accessibilityHidden(!interactive)
                    } else {
                        cardDetails(card)
                    }
                } else {
                    Color.clear
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func cardDetails(_ card: CampusMapCard) -> some View {
        CampusMapCardView(
            card: card,
            choose: { send(.choose(index: UInt32($0))) },
            open: { if let url = URL(string: $0) { openURL(url) } })
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
        if showCard { resetCardExpansion() }
        searchFocused = false
        query = ""
        firstResultHeight = 0
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
                ScrollView {
                    LazyVStack(spacing: 0) {
                        ForEach(answer.results, id: \.place) { result in
                            searchRow(result)
                                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { measured in
                                    if result.place == answer.results.first?.place, answer.query == query,
                                        measured > 0, abs(measured - firstResultHeight) > 0.5
                                    {
                                        firstResultHeight = measured
                                    }
                                }
                        }
                    }
                }
                .scrollDismissesKeyboard(.never)
                .id(answer.query)
            }
        } else {
            Color.clear
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

private struct CampusMapPanelHeader: ViewModifier {
    @Binding var height: CGFloat

    func body(content: Content) -> some View {
        content
            .padding(.horizontal, Theme.cardPadding)
            .padding(.vertical, 10)
            .fixedSize(horizontal: false, vertical: true)
            .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { measured in
                if measured > 0, abs(measured - height) > 0.5 { height = measured }
            }
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

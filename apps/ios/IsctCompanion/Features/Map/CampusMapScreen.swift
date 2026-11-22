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
    @Environment(AppModel.self) var model
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openURL) private var openURL
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    @Environment(\.verticalSizeClass) private var verticalSizeClass
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private(set) var load = CampusMapLoad.loading
    @State private var scene: CampusMapScene?
    /// Intents reach Core one after another, in the order sent.
    @State private var sending: Task<Void, Never>?
    @State private var location = CampusMapLocation()
    /// Taps of the position button: each picks the building the person is at.
    @State private var locates = 0
    /// Floors picked in the picker: each gives a tick and a cross-fade.
    @State private var levelPicks = 0
    /// The floor just picked, lit in the picker before Core answers.
    @State private var pickedLevel: String?
    /// Rooms, buildings and entrances picked on the map: each gives a tick.
    @State var mapPicks = 0
    /// The card's and the results' content, measured: each panel is as tall
    /// as its content up to a share of the window.
    @State private var cardHeight: CGFloat = 0
    @State private var cardHeaderHeight: CGFloat = 56
    /// Where the narrow window's card rests, and the finger's pull on it.
    @State private var cardDetent = CampusMapCardDetent.fit
    @State private var cardDrag: CGFloat = 0
    @State private var resultsHeight: CGFloat = 0
    @ScaledMetric(relativeTo: .body) var minimumResultHeight: CGFloat = 64
    @State private var railLeading: CGFloat?
    /// The map's visible part in window coordinates, with the request the
    /// layout that measured it answered.
    @State private var openFrame: CGRect?
    @State private var openLayout: CampusMapLayoutRequest?
    @State var query = ""
    @FocusState var searchFocused: Bool
    @State private var keyboardVisible = false
    @State private var pickingResult = false
    @State private var searchAnswer: CampusMapSearch?
    @State private var searchFailure: String?
    /// The building the camera is open for, and how its photos went.
    @State var cameraFor: String?
    @State var photoUploads: [String: FloorPhotoUploads] = [:]
    @State var photoFailure: String?

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
            searching: searching, card: showsCard ? cardHeight : nil, detent: cardDetent,
            dragging: cardDrag != 0, wide: wide)
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
        floorPhotoCamera(ZStack {
            FloorMapView(
                scene: scene,
                chrome: chrome,
                layoutRequest: layoutRequest,
                showsPosition: location.allowed,
                locates: locates,
                levelPicks: levelPicks,
                onCamera: { send(.camera(camera: $0)) },
                // Core ignores a tap on a floor it has already left.
                onTap: { selectOnMap(.tap(point: $0, levelId: $1, metresPerPoint: $2, roomsOnly: $3)) },
                onSelect: { selectOnMap(.select(place: $0)) },
                onLocated: { selectOnMap(.locate(point: $0)) },
                )
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
        // Each new place opens at its content's height.
        .onChange(of: scene?.card?.title) { _, _ in
            cardDetent = .fit
            cardDrag = 0
        }
        .onChange(of: location.allowed) { _, allowed in
            if allowed && location.asked { locates += 1 }
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
        .task(id: query) { await search(query) })
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
    func send(_ intent: CampusMapIntent, completion: ((CampusMapScene?) -> Void)? = nil) {
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
                cardPanel(card, window: size.height, adjustable: true)
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
                    cardPanel(card, window: size.height * 0.7, adjustable: false)
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
                rail(available: region.size.height - CampusMapChrome.controlSide - CampusMapChrome.controlGap)
                // On the card's top edge, so it rides along as the card moves.
                locateButton
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomTrailing)
                if let attribution = scene?.attribution, !attribution.isEmpty {
                    Text(attribution.joined(separator: " · "))
                        .font(.caption2)
                        .foregroundStyle(Theme.muted)
                        .padding(.horizontal, Theme.space4)
                        .padding(.vertical, Theme.space1)
                        .floatingGlass(in: Capsule())
                        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomLeading)
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
            searchField
            if searching {
                Button(t("common.cancel")) { endSearch() }
                    .font(.body.weight(.semibold))
                    .frame(minHeight: CampusMapChrome.controlSide)
                    .transition(.move(edge: .trailing).combined(with: .opacity))
            }
        }
    }

    /// Zooms to the building the person is at, or the nearest one, and
    /// selects it; asks for while-in-use permission on the first tap.
    private var locateButton: some View {
        Button {
            if location.allowed { locates += 1 } else { location.requestPermission() }
        } label: {
            Image(systemName: "scope")
                .font(.body.weight(.semibold))
                .foregroundStyle(location.allowed ? Theme.accent : Theme.ink)
                .frame(width: CampusMapChrome.controlSide, height: CampusMapChrome.controlSide)
                .floatingGlass(in: Circle())
        }
        .accessibilityLabel(t("campusMap.position"))
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
        let header = CampusMapChrome.railHeaderHeight
        let pickerHeight = available - 2 * header
        return Group {
            // Under the compass, the stack's button, then the floors.
            if let scene, !scene.levels.isEmpty, available >= 2 * header {
                VStack(spacing: CampusMapChrome.controlGap) {
                    stackButton(scene.stacked)
                    if pickerHeight >= CampusMapChrome.controlSide {
                        levels.frame(height: pickerHeight, alignment: .top)
                    }
                }
                .padding(.top, header)
            }
        }
        .frame(width: CampusMapChrome.controlSide, height: available, alignment: .top)
        .onGeometryChange(for: CGFloat.self) { $0.frame(in: .global).minX } action: { railLeading = $0 }
    }

    /// The building's floors as a stack seen at an angle, and back.
    private func stackButton(_ stacked: Bool) -> some View {
        let shape = RoundedRectangle(cornerRadius: Theme.radiusChip, style: .continuous)
        return Button {
            send(.stack(on: !stacked))
        } label: {
            Image(systemName: "square.3.layers.3d")
                .font(.body.weight(.semibold))
                .foregroundStyle(stacked ? Theme.accentInk : Theme.ink)
                .frame(width: CampusMapChrome.controlSide, height: CampusMapChrome.controlSide)
                .background(stacked ? Theme.accent : .clear)
                .clipShape(shape)
                .floatingGlass(in: shape)
                .contentShape(.interaction, shape)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(t(stacked ? "campusMap.stackOff" : "campusMap.stackOn"))
        .accessibilityAddTraits(stacked ? .isSelected : [])
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
    /// The lowest the card rests: its title and the start of its details.
    private static let peekHeight: CGFloat = 150

    /// The card's resting heights in a narrow window: about 150 pt, its
    /// content up to half the window, or up to the search bar.
    private func detentHeights(_ window: CGFloat) -> [CampusMapCardDetent: CGFloat] {
        let content = cardHeaderHeight + cardHeight
        let peek = min(content, max(cardHeaderHeight, Self.peekHeight))
        let fit = max(peek, min(content, window * 0.5))
        let full = max(fit, min(content, window - CampusMapChrome.controlSide - 4 * CampusMapChrome.controlGap))
        return [.peek: peek, .fit: fit, .full: full]
    }

    /// One level: the place, then everything about it. In a narrow window
    /// it rests at one of three heights, dragged by its top or stepped by
    /// tapping its grabber; beside the map it is as tall as its content.
    private func cardPanel(_ card: CampusMapCard, window: CGFloat, adjustable: Bool) -> some View {
        let heights = detentHeights(window)
        let resting = heights[cardDetent] ?? 0
        let lowest = heights[.peek] ?? 0
        let highest = heights[.full] ?? resting
        let raw = resting - cardDrag
        // Past either end the card follows the finger only a little.
        let shown = raw > highest
            ? highest + (raw - highest) * 0.2
            : raw < lowest ? lowest - (lowest - raw) * 0.2 : raw
        let height = adjustable ? shown : min(window, cardHeaderHeight + cardHeight)
        return glass(
            VStack(alignment: .leading, spacing: 0) {
                VStack(spacing: 0) {
                    if adjustable { cardGrabber(heights) }
                    cardHeader(card)
                }
                .padding(.top, adjustable ? 0 : Theme.space3)
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { measured in
                    if abs(measured - cardHeaderHeight) > 0.5 { cardHeaderHeight = measured }
                }
                .contentShape(Rectangle())
                .gesture(cardDragGesture(heights), including: adjustable ? .all : .subviews)
                ScrollView {
                    CampusMapCardView(
                        card: card,
                        choose: { send(.choose(index: UInt32($0))) },
                        select: { selectOnMap(.select(place: $0)) },
                        open: { if let url = URL(string: $0) { openURL(url) } },
                        photos: card.photoBuildingId.map { photoUploads[$0] ?? FloorPhotoUploads() },
                        takePhoto: {
                            if let id = card.photoBuildingId, UIImagePickerController.isSourceTypeAvailable(.camera) {
                                cameraFor = id
                            }
                        })
                        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { measured in
                            if abs(measured - cardHeight) > 0.5 { cardHeight = measured }
                        }
                }
                .scrollBounceBehavior(.basedOnSize)
                .scrollDisabled(adjustable && cardDetent == .peek)
            }
            .frame(height: max(height, CampusMapChrome.controlSide), alignment: .top))
    }

    private func cardGrabber(_ heights: [CampusMapCardDetent: CGFloat]) -> some View {
        Button {
            settleCard(cardDetent == .peek ? .fit : .peek)
        } label: {
            Capsule()
                .fill(Theme.muted.opacity(0.55))
                .frame(width: 36, height: 5)
                .frame(maxWidth: .infinity, minHeight: 16)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(t(cardDetent == .peek ? "campusMap.unfoldCard" : "campusMap.foldCard"))
        .accessibilityAdjustableAction { direction in
            switch direction {
            case .increment: settleCard(cardDetent == .peek ? .fit : .full)
            case .decrement: settleCard(cardDetent == .full ? .fit : .peek)
            @unknown default: break
            }
        }
    }

    /// Follows the finger, then settles on the height nearest to where the
    /// throw would have carried it.
    private func cardDragGesture(_ heights: [CampusMapCardDetent: CGFloat]) -> some Gesture {
        DragGesture(minimumDistance: 4, coordinateSpace: .global)
            .onChanged { cardDrag = $0.translation.height }
            .onEnded { drag in
                let projected = (heights[cardDetent] ?? 0) - drag.predictedEndTranslation.height
                let nearest = CampusMapCardDetent.allCases.min { first, second in
                    abs((heights[first] ?? 0) - projected) < abs((heights[second] ?? 0) - projected)
                }
                settleCard(nearest ?? cardDetent)
            }
    }

    private func settleCard(_ detent: CampusMapCardDetent) {
        withAnimation(cardSpring) {
            cardDetent = detent
            cardDrag = 0
        }
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

    func pick(_ result: CampusMapSearchResult) {
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

}

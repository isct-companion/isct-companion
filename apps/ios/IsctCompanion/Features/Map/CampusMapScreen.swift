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
    @State private var query = ""
    @State private var searchShown = false
    @State private var keyboardVisible = false
    @FocusState private var searchFocused: Bool
    @State private var searchAnswer: CampusMapSearch?
    @State private var searchFailure: String?

    private var results: [CampusMapSearchResult] {
        searchAnswer?.query == query ? (searchAnswer?.results ?? []) : []
    }

    private var layoutRequest: CampusMapLayoutRequest {
        CampusMapLayoutRequest(
            framing: scene?.framing?.key ?? 0, searchShown: searchShown, keyboardVisible: keyboardVisible)
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
                onTap: { send(.tap(point: $0, levelId: $1)) },
                onSelect: { send(.select(place: $0)) },
                onLocate: { location.requestPermission() })
                .ignoresSafeArea()
            VStack(spacing: 8) {
                topBar
                    .onGeometryChange(for: CGFloat.self) { $0.frame(in: .global).maxY } action: { chrome.top = $0 }
                ZStack {
                    mapControls
                        .opacity(searchShown ? 0 : 1)
                        .allowsHitTesting(!searchShown)
                        .accessibilityHidden(searchShown)
                    searchList
                        .opacity(searchShown ? 1 : 0)
                        .allowsHitTesting(searchShown)
                        .accessibilityHidden(!searchShown)
                }
            }
            .padding(.horizontal, CampusMapChrome.pageInset)
            .padding(.bottom, 8)
        }
        .background(Theme.bg)
        .sensoryFeedback(.selection, trigger: scene?.levelId)
        .onChange(of: searchFocused) { _, focused in
            if focused { searchShown = true }
        }
        .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillShowNotification)) { _ in
            keyboardVisible = true
        }
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
        .task { await open() }
        .task(id: query) { await search(query) }
        // Also when dismissed before the open finished: Core drops it.
        .onDisappear { model.campusMapClose() }
    }

    /// Keep both content containers mounted: showing search never replaces
    /// the field or changes its structural identity as the keyboard lays out.
    private var mapControls: some View {
        VStack(spacing: 8) {
            HStack(alignment: .top) {
                Spacer(minLength: 0)
                VStack(spacing: 0) {
                    Color.clear
                        .frame(height: CampusMapChrome.railHeaderHeight)
                        .allowsHitTesting(false)
                    if let scene, !scene.levels.isEmpty {
                        LevelPicker(levels: scene.levels, selected: scene.levelId, marked: scene.markedLevelId) {
                            send(.level(id: $0))
                        }
                    }
                    Spacer(minLength: 0)
                }
                .frame(width: CampusMapChrome.controlSide)
                .onGeometryChange(for: CGFloat.self) { $0.frame(in: .global).minX } action: {
                    chrome.trailing = $0
                }
            }
            .frame(maxHeight: .infinity, alignment: .top)
            bottomPanel
                .background {
                    CampusMapPanelLayoutProbe(request: layoutRequest) { request, top in
                        guard request == layoutRequest else { return }
                        let bottom = request.searchShown ? CGFloat.greatestFiniteMagnitude : top
                        if chrome.bottom != bottom { chrome.bottom = bottom }
                        if chrome.layout != request { chrome.layout = request }
                    }
                    .allowsHitTesting(false)
                }
        }
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
                    .onSubmit { searchFocused = false }
                if searchShown {
                    Button {
                        cancelSearch()
                    } label: {
                        Image(systemName: "xmark.circle.fill").foregroundStyle(Theme.muted)
                    }
                    .accessibilityLabel(t("common.cancel"))
                }
            }
            .padding(.horizontal, 12)
            .frame(minHeight: 44)
            // Keep the editable subtree on one material implementation.
            .background(.regularMaterial, in: Capsule())
        }
    }

    /// Asks Core for the text's places; an answer to text the person has
    /// since changed is dropped (Core echoes the text it answered). The map
    /// does not move while they type.
    private func search(_ text: String) async {
        guard !Task.isCancelled, text == query else { return }
        searchAnswer = nil
        searchFailure = nil
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        let answer = await model.campusMapSearch(text)
        guard !Task.isCancelled, searchShown, text == query else { return }
        switch answer {
        case let .success(found) where found.query == query:
            searchAnswer = found
        case .success:
            break
        case let .failure(failure):
            searchFailure = failure.message
        }
    }

    private func cancelSearch() {
        searchFocused = false
        searchShown = false
        query = ""
        searchAnswer = nil
        searchFailure = nil
    }

    private func pick(_ result: CampusMapSearchResult) {
        cancelSearch()
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
        // Offset changes during keyboard layout must not resign the field.
        .scrollDismissesKeyboard(.never)
        .simultaneousGesture(
            DragGesture(minimumDistance: 10).onChanged { _ in
                if searchFocused { searchFocused = false }
            })
        .background(Theme.surface, in: RoundedRectangle(cornerRadius: Theme.radiusPanel, style: .continuous))
        .clipShape(RoundedRectangle(cornerRadius: Theme.radiusPanel, style: .continuous))
        .frame(maxHeight: .infinity)
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
                        expanded: $expanded,
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
}

/// Two heights, clipped from one content tree. Only the grabber
/// and title initiate a drag; details and the close button keep their own touches.
private struct CampusMapCardView: View {
    let card: CampusMapCard
    @Binding var expanded: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var detailsHeight: CGFloat = 0
    @GestureState(resetTransaction: Transaction(animation: .smooth(duration: Theme.motionSlow)))
    private var dragTranslation: CGFloat? = nil
    let close: () -> Void
    let choose: (Int) -> Void
    let open: (String) -> Void

    private func expansion(at translation: CGFloat) -> CGFloat {
        let start: CGFloat = expanded ? 1 : 0
        guard detailsHeight > 0 else { return start }
        return min(1, max(0, start - translation / detailsHeight))
    }

    var body: some View {
        CampusMapCardLayout(expansion: expansion(at: dragTranslation ?? 0)) {
            header
            details
                .padding(.horizontal, Theme.cardPadding)
                .padding(.top, 2)
                .padding(.bottom, Theme.cardPadding)
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { detailsHeight = $0 }
                .allowsHitTesting(expanded && dragTranslation == nil)
                .accessibilityHidden(!expanded)
        }
        .clipped()
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Theme.surface, in: RoundedRectangle(cornerRadius: Theme.radiusPanel, style: .continuous))
        .shadow(color: .black.opacity(0.12), radius: 8, y: 2)
        .accessibilityElement(children: .contain)
        .transaction { transaction in
            if reduceMotion { transaction.animation = nil }
        }
    }

    private var header: some View {
        VStack(spacing: 0) {
            Capsule()
                .fill(Theme.line)
                .frame(width: 36, height: 5)
                .frame(maxWidth: .infinity)
                .frame(height: 24)
                .contentShape(Rectangle())
                .onTapGesture { setExpanded(!expanded) }
                .gesture(resizeGesture)
                .accessibilityHidden(true)
            HStack(alignment: .center, spacing: 8) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(card.title ?? t("campusMap.entrance"))
                        .font(.headline)
                        .foregroundStyle(Theme.ink)
                        .lineLimit(2)
                    Text(subtitle)
                        .font(.subheadline)
                        .foregroundStyle(Theme.muted)
                        .lineLimit(1)
                }
                .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                .contentShape(Rectangle())
                .onTapGesture { setExpanded(!expanded) }
                .gesture(resizeGesture)
                .accessibilityElement(children: .combine)
                .accessibilityAddTraits(.isButton)
                .accessibilityValue(t(expanded ? "campusMap.cardExpanded" : "campusMap.cardCollapsed"))
                .accessibilityAction { setExpanded(!expanded) }
                .accessibilityAction(named: t(expanded ? "campusMap.foldCard" : "campusMap.unfoldCard")) {
                    setExpanded(!expanded)
                }
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
            .padding(.horizontal, Theme.cardPadding)
        }
        .padding(.bottom, 8)
    }

    private var details: some View {
        VStack(alignment: .leading, spacing: 10) {
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

    private var resizeGesture: some Gesture {
        DragGesture(minimumDistance: 10, coordinateSpace: .global)
            .updating($dragTranslation) { drag, translation, transaction in
                guard abs(drag.translation.height) > abs(drag.translation.width) else { return }
                transaction.animation = nil
                translation = drag.translation.height
            }
            .onEnded { drag in
                guard abs(drag.translation.height) > abs(drag.translation.width) else { return }
                setExpanded(expansion(at: drag.translation.height) >= 0.5)
            }
    }

    private func setExpanded(_ open: Bool) {
        withAnimation(reduceMotion ? nil : .smooth(duration: Theme.motionSlow)) {
            expanded = open
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

/// SwiftUI has no in-page two-height card layout. Measure both children in
/// this pass so new content never uses the previous selection's endpoint size.
private struct CampusMapCardLayout: SwiftUI.Layout {
    var expansion: CGFloat

    var animatableData: CGFloat {
        get { expansion }
        set { expansion = newValue }
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let sizes = measure(width: proposal.width, subviews: subviews)
        return CGSize(
            width: proposal.width ?? max(sizes.header.width, sizes.details.width),
            height: sizes.header.height + sizes.details.height * expansion)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let sizes = measure(width: bounds.width, subviews: subviews)
        subviews[0].place(
            at: bounds.origin, anchor: .topLeading,
            proposal: ProposedViewSize(width: bounds.width, height: sizes.header.height))
        subviews[1].place(
            at: CGPoint(x: bounds.minX, y: bounds.minY + sizes.header.height), anchor: .topLeading,
            proposal: ProposedViewSize(width: bounds.width, height: sizes.details.height))
    }

    private func measure(width: CGFloat?, subviews: Subviews) -> (header: CGSize, details: CGSize) {
        let proposal = ProposedViewSize(width: width, height: nil)
        return (subviews[0].sizeThatFits(proposal), subviews[1].sizeThatFits(proposal))
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

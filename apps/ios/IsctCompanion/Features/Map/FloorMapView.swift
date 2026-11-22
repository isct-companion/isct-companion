import IsctCore
import MapKit
import SwiftUI

/// The part of the map the page's own controls leave, in window
/// coordinates: under the search bar (and the results), over the card,
/// beside the leading column, and the control rail's leading edge. The rail
/// stays reserved when a floor picker is not shown.
struct CampusMapChrome: Equatable {
    static let controlSide: CGFloat = 44
    static let controlGap: CGFloat = Theme.space4
    static let pageInset: CGFloat = Theme.space6
    /// The compass, at the top of the rail.
    static let railHeaderHeight = controlSide + controlGap

    var top: CGFloat = 0
    var bottom: CGFloat = .greatestFiniteMagnitude
    var leading: CGFloat = 0
    var trailing: CGFloat?
    /// The request a completed panel layout pass measured these edges for.
    var layout: CampusMapLayoutRequest?
}

/// The narrow window's card heights: its title alone, its content up to
/// half the window, or up to the search bar.
enum CampusMapCardDetent: CaseIterable, Hashable {
    case peek, fit, full
}

/// What the map waits on before an explicit framing: the framing itself and
/// the panels it is shown beside.
struct CampusMapLayoutRequest: Equatable {
    let framing: UInt64
    let keyboardVisible: Bool
    let searching: Bool
    /// The card's content height while it shows, and where it rests.
    let card: CGFloat?
    let detent: CampusMapCardDetent
    /// A finger is pulling the card: the visible part is moving.
    let dragging: Bool
    /// The panels sit in a column along the leading edge.
    let wide: Bool

    var canFrame: Bool { !keyboardVisible && !dragging }
}

/// UIKit is asked to lay out after every request update, even at an equal
/// size. Geometry-change callbacks cannot acknowledge an unchanged frame.
struct CampusMapPanelLayoutProbe: UIViewRepresentable {
    let request: CampusMapLayoutRequest
    let onLayout: (CampusMapLayoutRequest, CGRect) -> Void

    func makeUIView(context: Context) -> PanelLayoutView { PanelLayoutView() }

    func updateUIView(_ view: PanelLayoutView, context: Context) {
        view.request = request
        view.onLayout = onLayout
        view.setNeedsLayout()
    }

    final class PanelLayoutView: UIView {
        var request: CampusMapLayoutRequest?
        var onLayout: ((CampusMapLayoutRequest, CGRect) -> Void)?

        override func layoutSubviews() {
            super.layoutSubviews()
            guard let request, window != nil else { return }
            // Publish after the containing SwiftUI layout has finished, not
            // while it is updating its native views or measuring the panel.
            DispatchQueue.main.async { [weak self] in
                guard let self, window != nil, self.request == request else { return }
                onLayout?(request, convert(bounds, to: nil))
            }
        }
    }
}

/// Apple's base map with Core's scene drawn over it by role. Core decides
/// every layer and mark; this view only batches and paints them, moves the
/// camera where Core frames, and reports the person's own moves of it over
/// the part of the map the controls leave visible. In the stack it is seen
/// at an angle, and the floors are drawn over the map (`StackOverlayView`).
struct FloorMapView: UIViewRepresentable {
    let scene: CampusMapScene?
    let chrome: CampusMapChrome
    let layoutRequest: CampusMapLayoutRequest
    let showsPosition: Bool
    /// Rises each time the person asks where they are; the next position
    /// MapKit has goes to `onLocated` once, and Core picks the building.
    let locates: Int
    /// Rises with each floor picked in the picker: that floor change
    /// cross-fades, while one the camera causes is drawn as it comes.
    let levelPicks: Int
    /// Where the camera settled after the person moved it. A move of this
    /// view's own, for a framing, is never reported: Core does not wait for
    /// it to land.
    let onCamera: (CampusMapCamera) -> Void
    /// A tap off the marks, the floor drawn under it, the map's metres per
    /// point there, and whether it is the quick one sent before a double
    /// tap is ruled out (Core then selects only a room).
    let onTap: (CampusMapPoint, String?, Double, Bool) -> Void
    let onSelect: (CampusMapPlace) -> Void
    let onLocated: (CampusMapPoint) -> Void

    /// Room between the visible part's edges and what a framing shows.
    private static let framingMargin: CGFloat = 16

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeUIView(context: Context) -> MeasuredMapView {
        let map = MeasuredMapView()
        // MapKit's default camera would show before Core and the panel are ready.
        map.alpha = 0
        let coordinator = context.coordinator
        let configuration = MKStandardMapConfiguration(elevationStyle: .flat, emphasisStyle: .muted)
        configuration.pointOfInterestFilter = .excludingAll
        map.preferredConfiguration = configuration
        map.isPitchEnabled = false
        map.showsCompass = false
        // Framing uses edgePadding only. Changing MapKit's layout margins
        // also changes its camera alignment, even without a new Core command.
        map.layoutMargins = .zero
        // Safe areas are included explicitly in visible(in:).
        map.insetsLayoutMarginsFromSafeArea = false
        map.delegate = coordinator
        map.register(FloorMarkView.self, forAnnotationViewWithReuseIdentifier: FloorMarkView.reuse)
        // A double tap zooms, which MapKit does. The quick tap answers the
        // moment the finger lifts and can only pick a room; the full tap
        // waits for the double tap to fail, so a zoom never picks a
        // building or closes the card.
        let quick = UITapGestureRecognizer(target: coordinator, action: #selector(Coordinator.tappedQuickly(_:)))
        quick.cancelsTouchesInView = false
        quick.delaysTouchesBegan = false
        quick.delaysTouchesEnded = false
        quick.delegate = coordinator
        map.addGestureRecognizer(quick)
        let double = UITapGestureRecognizer(target: coordinator, action: #selector(Coordinator.doubleTapped(_:)))
        double.numberOfTapsRequired = 2
        double.cancelsTouchesInView = false
        double.delaysTouchesEnded = false
        double.delegate = coordinator
        let tap = UITapGestureRecognizer(target: coordinator, action: #selector(Coordinator.tapped(_:)))
        tap.cancelsTouchesInView = false
        tap.delegate = coordinator
        tap.require(toFail: double)
        tap.delaysTouchesBegan = false
        tap.delaysTouchesEnded = false
        map.addGestureRecognizer(double)
        map.addGestureRecognizer(tap)
        // MapKit has no public camera-change reason. Observe contacts with
        // our own passive recognizer instead of inspecting its view tree.
        let contact = MapContactRecognizer(target: coordinator, action: #selector(Coordinator.contactChanged(_:)))
        contact.cancelsTouchesInView = false
        contact.delaysTouchesBegan = false
        contact.delaysTouchesEnded = false
        contact.delegate = coordinator
        map.addGestureRecognizer(contact)
        coordinator.contact = contact
        // Over the map's own content, under the compass.
        let stack = StackOverlayView(frame: map.bounds)
        stack.onSelect = { [weak coordinator] id in coordinator?.onSelect(.entrance(id: id)) }
        map.addSubview(stack)
        coordinator.stackView = stack
        coordinator.ticker.tick = { [weak coordinator, weak map] in
            guard let coordinator, let map else { return }
            coordinator.raise(map)
        }
        let compass = MKCompassButton(mapView: map)
        compass.compassVisibility = .adaptive
        map.addSubview(compass)
        coordinator.compass = compass
        map.laidOut = { [weak coordinator, weak map] in
            guard let coordinator, let map else { return }
            coordinator.layOut(map)
            coordinator.applyFraming(to: map)
            coordinator.raise(map)
        }
        return map
    }

    func updateUIView(_ map: MeasuredMapView, context: Context) {
        let coordinator = context.coordinator
        coordinator.onCamera = onCamera
        coordinator.onTap = onTap
        coordinator.onSelect = onSelect
        coordinator.animatesCamera = !context.environment.accessibilityReduceMotion
        coordinator.chrome = chrome
        coordinator.layoutRequest = layoutRequest
        coordinator.layOut(map)
        if map.showsUserLocation != showsPosition { map.showsUserLocation = showsPosition }
        coordinator.onLocated = onLocated
        if locates != coordinator.locates {
            coordinator.locates = locates
            coordinator.locating = true
            coordinator.deliverPosition(map)
        }
        let appearance = "\(context.environment.colorScheme)|\(context.environment.isctTheme)"
        coordinator.prepareAppearance(appearance, on: map)
        // The pick lands with a later scene: fade once its floor is drawn.
        if levelPicks != coordinator.levelPicks {
            coordinator.levelPicks = levelPicks
            coordinator.fadesNextFloor = !context.environment.accessibilityReduceMotion
        }
        if coordinator.drawnPage != scene?.page || coordinator.drawnRevision != scene?.revision {
            if coordinator.fadesNextFloor, scene?.levelId != coordinator.levelId {
                coordinator.fadesNextFloor = false
                coordinator.crossFade(map)
            }
            coordinator.draw(scene?.layers ?? [], on: map)
            coordinator.draw(scene?.marks ?? [], on: map)
            coordinator.drawnPage = scene?.page
            coordinator.drawnRevision = scene?.revision
        }
        coordinator.levelId = scene?.levelId
        if let framing = scene?.framing,
            framing.key > coordinator.appliedFraming,
            framing.key != coordinator.pendingFraming?.key
        {
            coordinator.pendingFraming = framing
        }
        coordinator.stackScene = scene
        coordinator.tilt(map, stacked: scene?.stacked == true)
        coordinator.applyFraming(to: map)
        coordinator.raise(map)
    }

    final class Coordinator: NSObject, MKMapViewDelegate, UIGestureRecognizerDelegate {
        var onCamera: (CampusMapCamera) -> Void = { _ in }
        var onTap: (CampusMapPoint, String?, Double, Bool) -> Void = { _, _, _, _ in }
        var onSelect: (CampusMapPlace) -> Void = { _ in }
        var animatesCamera = true
        var chrome = CampusMapChrome()
        var layoutRequest: CampusMapLayoutRequest?
        var locates = 0
        /// A position was asked for and has not been delivered yet.
        var locating = false
        var onLocated: (CampusMapPoint) -> Void = { _ in }
        var levelPicks = 0
        var fadesNextFloor = false
        var levelId: String?
        var drawnPage: UInt64?
        var drawnRevision: UInt64?
        var appliedFraming: UInt64 = 0
        var pendingFraming: CampusMapFraming?
        weak var compass: MKCompassButton?
        weak var contact: MapContactRecognizer?
        weak var stackView: StackOverlayView?
        /// The scene drawn, for the stack, which follows the camera.
        var stackScene: CampusMapScene?
        let ticker = StackTicker()
        /// The camera is tilted for the stack.
        private(set) var stacked = false
        /// Where the centre of the view was on the ground when the person's
        /// touch began: in the stack, a turn or tilt that leaves it in place
        /// is no move to another building.
        private var movedFrom: CLLocationCoordinate2D?
        /// The framing this view is moving the camera for: a move made
        /// meanwhile is not the person's, unless a finger interrupts it.
        /// Native animations run one at a time; a newer framing waits while
        /// the current move settles.
        private var activeFraming: UInt64?
        private var announced = false
        private var appearance: String?
        private var overlays: [String: MKOverlay] = [:]
        private var marks: [String: FloorMark] = [:]

        deinit { ticker.stop() }

        // MARK: Visible part

        /// The map less the page's controls, in the map's own points.
        func visible(in map: MKMapView) -> CGRect {
            let bounds = map.bounds
            let inMap = { (point: CGPoint) in map.window == nil ? point : map.convert(point, from: nil) }
            let top = max(inMap(CGPoint(x: 0, y: chrome.top)).y, bounds.minY + map.safeAreaInsets.top)
            let bottom = min(inMap(CGPoint(x: 0, y: min(chrome.bottom, 1e6))).y, bounds.maxY - map.safeAreaInsets.bottom)
            let trailing = chrome.trailing.map { min(inMap(CGPoint(x: $0, y: 0)).x, bounds.maxX) } ?? bounds.maxX
            let leading = max(inMap(CGPoint(x: chrome.leading, y: 0)).x, bounds.minX + map.safeAreaInsets.left)
            let right = min(trailing, bounds.maxX - map.safeAreaInsets.right)
            return CGRect(
                x: leading, y: top,
                width: max(right - leading, 1), height: max(bottom - top, 1))
        }

        /// The compass sits at the top of the same rail as the floor picker;
        /// the stack covers the map.
        func layOut(_ map: MKMapView) {
            guard map.bounds.width > 0, map.bounds.height > 0 else { return }
            let side = CampusMapChrome.controlSide
            let x = map.bounds.maxX - map.safeAreaInsets.right - side - CampusMapChrome.pageInset
            // At the top of the map's open part, under the search bar.
            let open = map.window == nil ? chrome.top : map.convert(CGPoint(x: 0, y: chrome.top), from: nil).y
            let compassY = max(open, map.bounds.minY + map.safeAreaInsets.top)
            compass?.frame = CGRect(x: x, y: compassY, width: side, height: side)
            if stackView?.frame != map.bounds { stackView?.frame = map.bounds }
        }

        // MARK: Framing

        func applyFraming(to map: MKMapView) {
            guard let layoutRequest, layoutRequest.canFrame, chrome.layout == layoutRequest,
                let framing = pendingFraming, framing.key == layoutRequest.framing,
                activeFraming == nil, contact?.fingersDown != true, chrome.trailing != nil,
                map.window != nil
            else { return }
            let viewport = visible(in: map)
            let minSide = 2 * FloorMapView.framingMargin
            guard viewport.width > minSide, viewport.height > minSide else { return }
            let animated = appliedFraming != 0 && animatesCamera
            pendingFraming = nil
            if framing.reveal {
                reveal(framing, in: viewport, on: map, animated: animated)
            } else {
                frame(framing, in: viewport, on: map, animated: animated)
            }
        }

        /// Pans the least that brings the bounds inside the visible part,
        /// keeping the zoom; centres them when they are larger than it.
        private func reveal(_ framing: CampusMapFraming, in visible: CGRect, on map: MKMapView, animated: Bool) {
            let bounds = framing.bounds
            let corners = [
                (bounds.north, bounds.west), (bounds.north, bounds.east),
                (bounds.south, bounds.east), (bounds.south, bounds.west),
            ].map { map.convert(CLLocationCoordinate2D(latitude: $0.0, longitude: $0.1), toPointTo: map) }
            let xs = corners.map(\.x), ys = corners.map(\.y)
            let target = CGRect(
                x: xs.min() ?? 0, y: ys.min() ?? 0,
                width: (xs.max() ?? 0) - (xs.min() ?? 0), height: (ys.max() ?? 0) - (ys.min() ?? 0))
            let room = visible.insetBy(dx: FloorMapView.framingMargin, dy: FloorMapView.framingMargin)
            let shift = { (low: CGFloat, high: CGFloat, from: CGFloat, to: CGFloat) -> CGFloat in
                if high - low > to - from { return (low + high) / 2 - (from + to) / 2 }
                if low < from { return low - from }
                if high > to { return high - to }
                return 0
            }
            let dx = shift(target.minX, target.maxX, room.minX, room.maxX)
            let dy = shift(target.minY, target.maxY, room.minY, room.maxY)
            appliedFraming = framing.key
            activeFraming = framing.key
            announced = false
            if dx != 0 || dy != 0 {
                if map.userTrackingMode != .none { map.setUserTrackingMode(.none, animated: false) }
                let center = map.convert(map.centerCoordinate, toPointTo: map)
                let moved = map.convert(CGPoint(x: center.x + dx, y: center.y + dy), toCoordinateFrom: map)
                map.setCenter(moved, animated: animated)
            }
            DispatchQueue.main.async { [weak self, weak map] in
                guard let self, let map, activeFraming == framing.key, !announced else { return }
                finishFraming(on: map)
            }
        }

        /// The floor shown now fades out over the one drawn next, which
        /// MapKit renders in tiles a moment later. Only a floor picked from
        /// the picker fades, when no finger moves the map under the picture.
        func crossFade(_ map: MKMapView) {
            guard contact?.fingersDown != true, map.alpha > 0,
                let picture = map.snapshotView(afterScreenUpdates: false)
            else { return }
            picture.isUserInteractionEnabled = false
            picture.frame = map.bounds
            // Over the floor but under the native controls.
            if let compass { map.insertSubview(picture, belowSubview: compass) } else { map.addSubview(picture) }
            UIView.animate(withDuration: 0.18, delay: 0.04, options: [.curveEaseOut, .allowUserInteraction]) {
                picture.alpha = 0
            } completion: { _ in
                picture.removeFromSuperview()
            }
        }

        private func frame(_ framing: CampusMapFraming, in visible: CGRect, on map: MKMapView, animated: Bool) {
            let first = appliedFraming == 0
            let bounds = framing.bounds
            let corner = MKMapPoint(CLLocationCoordinate2D(latitude: bounds.north, longitude: bounds.west))
            let opposite = MKMapPoint(CLLocationCoordinate2D(latitude: bounds.south, longitude: bounds.east))
            let rect = MKMapRect(
                x: min(corner.x, opposite.x), y: min(corner.y, opposite.y),
                width: abs(opposite.x - corner.x), height: abs(opposite.y - corner.y))
            let margin = FloorMapView.framingMargin
            let padding = UIEdgeInsets(
                top: visible.minY + margin, left: visible.minX + margin,
                bottom: map.bounds.maxY - visible.maxY + margin,
                right: map.bounds.maxX - visible.maxX + margin)
            if map.userTrackingMode != .none { map.setUserTrackingMode(.none, animated: false) }
            appliedFraming = framing.key
            activeFraming = framing.key
            announced = false
            // The stack is framed at its angle, and a floor after it from
            // above again; MapKit fits a box only from above.
            let pitch = stacked ? CampusMapStack.pitch : 0
            if pitch > 0 || map.camera.pitch > 0.5 {
                // A first framing puts the camera on campus from above, which
                // gives the map's scale to work the angle out from.
                if first { map.setVisibleMapRect(rect, edgePadding: padding, animated: false) }
                if let camera = angledCamera(bounds, in: visible, on: map, pitch: pitch) {
                    map.setCamera(camera, animated: animated)
                } else {
                    map.setVisibleMapRect(rect, edgePadding: padding, animated: animated)
                }
            } else {
                map.setVisibleMapRect(rect, edgePadding: padding, animated: animated)
            }
            // A no-op framing need not produce region callbacks. Give MapKit
            // the current update before acknowledging it, rather than assuming
            // callbacks must occur synchronously inside setVisibleMapRect.
            DispatchQueue.main.async { [weak self, weak map] in
                guard let self, let map, activeFraming == framing.key, !announced else { return }
                finishFraming(on: map)
            }
        }

        /// A camera at `pitch` that shows the bounds, and in the stack its
        /// lowest and highest floors, in the visible part, keeping the map's
        /// heading.
        private func angledCamera(
            _ bounds: CampusMapBounds, in visible: CGRect, on map: MKMapView, pitch: CLLocationDegrees
        ) -> MKMapCamera? {
            // The focal length, from the map's scale across the centre of
            // the view, which is the camera's distance over it.
            let centre = CGPoint(x: map.bounds.midX, y: map.bounds.midY)
            let span: CGFloat = 100
            let here = map.convert(centre, toCoordinateFrom: map)
            let across = map.convert(CGPoint(x: centre.x + span, y: centre.y), toCoordinateFrom: map)
            let metresPerPoint = MKMapPoint(here).distance(to: MKMapPoint(across)) / Double(span)
            let distance = map.camera.centerCoordinateDistance
            guard metresPerPoint > 0, distance > 0 else { return nil }
            let floors = stacked ? stackScene?.stack ?? [] : []
            let heights = [floors.first?.baseM ?? 0, floors.last?.baseM ?? 0]
            let margin = FloorMapView.framingMargin
            guard
                let fit = StackFraming.fit(
                    bounds, heights: heights, focal: distance / metresPerPoint, view: map.bounds.size,
                    room: visible.insetBy(dx: margin, dy: margin), pitch: pitch, heading: map.camera.heading)
            else { return nil }
            return MKMapCamera(
                lookingAtCenter: fit.center, fromDistance: fit.distance, pitch: pitch, heading: map.camera.heading)
        }

        /// The stack is seen at an angle and may be turned and tilted; a
        /// single floor is seen from above. A framing for this scene sets
        /// the angle itself; otherwise the camera tilts where it is. Either
        /// is this view's own move, which is reported to no one.
        func tilt(_ map: MKMapView, stacked: Bool) {
            guard stacked != self.stacked else { return }
            self.stacked = stacked
            map.isPitchEnabled = stacked
            // Apple's own buildings would rise through the floors.
            map.showsBuildings = !stacked
            movedFrom = nil
            guard pendingFraming == nil, appliedFraming != 0 else { return }
            guard let camera = map.camera.copy() as? MKMapCamera else { return }
            camera.pitch = stacked ? CampusMapStack.pitch : 0
            let key = appliedFraming
            activeFraming = key
            announced = false
            map.setCamera(camera, animated: animatesCamera)
            DispatchQueue.main.async { [weak self, weak map] in
                guard let self, let map, activeFraming == key, !announced else { return }
                finishFraming(on: map)
            }
        }

        /// Draws the stack for the camera as it is now, or takes it away.
        func raise(_ map: MKMapView) {
            guard let stackView else { return }
            guard let scene = stackScene, scene.stacked, map.bounds.width > 0, map.bounds.height > 0,
                let sight = StackSight(map: map, in: stackView)
            else {
                if !stackView.isHidden { stackView.show(nil, entrances: []) }
                return
            }
            let (picture, entrances) = StackPicture.make(scene, sight: sight, visible: visible(in: map))
            stackView.show(picture, entrances: entrances)
        }

        // MARK: Camera and taps

        /// Every step of the camera, the person's and MapKit's own.
        func mapViewDidChangeVisibleRegion(_ map: MKMapView) {
            raise(map)
        }

        func mapView(_ map: MKMapView, regionWillChangeAnimated animated: Bool) {
            // MapKit need not report every step of a move it animates.
            if stacked { ticker.start() }
            if contact?.fingersDown == true {
                activeFraming = nil
            } else if activeFraming != nil {
                announced = true
            }
        }

        func mapView(_ map: MKMapView, regionDidChangeAnimated animated: Bool) {
            ticker.stop()
            raise(map)
            finishFraming(on: map, byUser: activeFraming == nil || contact?.fingersDown == true)
        }

        @objc func contactChanged(_ recognizer: MapContactRecognizer) {
            if recognizer.fingersDown {
                // Latch the interruption through deceleration after release.
                activeFraming = nil
                if recognizer.state == .began, let map = recognizer.view as? MKMapView {
                    movedFrom = map.centerCoordinate
                }
            } else if let map = recognizer.view as? MKMapView {
                DispatchQueue.main.async { [weak self, weak map] in
                    guard let self, let map else { return }
                    applyFraming(to: map)
                }
            }
        }

        private func finishFraming(on map: MKMapView, byUser: Bool = false) {
            if activeFraming != nil, map.alpha == 0 { map.alpha = 1 }
            activeFraming = nil
            if byUser { report(map) }
            // Avoid starting another native move from inside its delegate.
            DispatchQueue.main.async { [weak self, weak map] in
                guard let self, let map else { return }
                applyFraming(to: map)
            }
        }

        /// The visible part's corners on the ground: a turned map gives a
        /// turned box. In the stack, seen at an angle, only its near half:
        /// the far half reaches far off and would weigh far more than it
        /// shows. A turn or tilt of the stack is not reported.
        private func report(_ map: MKMapView) {
            guard appliedFraming != 0, map.window != nil else { return }
            let from = movedFrom
            movedFrom = nil
            if stacked, let from {
                let at = map.convert(from, toPointTo: map)
                let moved = hypot(at.x - map.bounds.midX, at.y - map.bounds.midY)
                if moved < CampusMapStack.panPoints { return }
            }
            var visible = visible(in: map)
            if stacked {
                visible = CGRect(
                    x: visible.minX, y: visible.midY, width: visible.width, height: visible.height / 2)
            }
            let at = { (x: CGFloat, y: CGFloat) -> CampusMapPoint in
                let coordinate = map.convert(CGPoint(x: x, y: y), toCoordinateFrom: map)
                return CampusMapPoint(latitude: coordinate.latitude, longitude: coordinate.longitude)
            }
            let camera = CampusMapCamera(
                topLeft: at(visible.minX, visible.minY),
                topRight: at(visible.maxX, visible.minY),
                bottomRight: at(visible.maxX, visible.maxY),
                bottomLeft: at(visible.minX, visible.maxY))
            DispatchQueue.main.async { [onCamera] in onCamera(camera) }
        }

        func gestureRecognizer(
            _ gestureRecognizer: UIGestureRecognizer,
            shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer
        ) -> Bool { true }

        func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
            if gestureRecognizer is MapContactRecognizer { return true }
            var hit = touch.view
            while let view = hit, view !== gestureRecognizer.view {
                if view is UIControl || view is MKAnnotationView || view is MKCompassButton {
                    return false
                }
                hit = view.superview
            }
            return true
        }

        /// Hands the position on once, as soon as MapKit has a fix.
        func deliverPosition(_ map: MKMapView) {
            guard locating, let position = map.userLocation.location,
                position.horizontalAccuracy >= 0, -position.timestamp.timeIntervalSinceNow < 60
            else { return }
            locating = false
            onLocated(CampusMapPoint(latitude: position.coordinate.latitude, longitude: position.coordinate.longitude))
        }

        func mapView(_ map: MKMapView, didUpdate userLocation: MKUserLocation) {
            deliverPosition(map)
        }

        @objc func doubleTapped(_ recognizer: UITapGestureRecognizer) {}

        @objc func tappedQuickly(_ recognizer: UITapGestureRecognizer) { send(recognizer, roomsOnly: true) }

        @objc func tapped(_ recognizer: UITapGestureRecognizer) { send(recognizer, roomsOnly: false) }

        private func send(_ recognizer: UITapGestureRecognizer, roomsOnly: Bool) {
            guard recognizer.state == .ended, let map = recognizer.view as? MKMapView else { return }
            let location = recognizer.location(in: map)
            let coordinate = map.convert(location, toCoordinateFrom: map)
            // The map's scale here, over a few points across the tap.
            let span: CGFloat = 20
            let across = map.convert(CGPoint(x: location.x + span, y: location.y), toCoordinateFrom: map)
            let metres = MKMapPoint(coordinate).distance(to: MKMapPoint(across))
            onTap(
                CampusMapPoint(latitude: coordinate.latitude, longitude: coordinate.longitude),
                levelId, metres / Double(span), roomsOnly)
        }

        /// A tap on a mark, or VoiceOver activating one.
        func mapView(_ map: MKMapView, didSelect view: MKAnnotationView) {
            if let mark = view.annotation as? FloorMark {
                switch mark.record.kind {
                case let .entrance(id, _, _): onSelect(.entrance(id: id))
                case let .room(unitId, _, _): onSelect(.unit(id: unitId))
                case let .building(buildingId, _): onSelect(.building(id: buildingId))
                case .amenity: break
                }
            }
            map.deselectAnnotation(view.annotation, animated: false)
        }

        // MARK: Drawing

        /// Layer ids identify their content, so unchanged geometry stays in
        /// MapKit even when another part of the scene changes.
        func draw(_ layers: [CampusMapLayer], on map: MKMapView) {
            let wanted = Set(layers.map(\.id))
            let removed = overlays.filter { !wanted.contains($0.key) }
            if !removed.isEmpty { map.removeOverlays(Array(removed.values)) }
            removed.keys.forEach { overlays.removeValue(forKey: $0) }

            for (index, layer) in layers.enumerated() where overlays[layer.id] == nil {
                let overlay: MKOverlay
                switch layer.role {
                case .detailLine, .wallLine:
                    let lines = layer.lines.map { line -> MKPolyline in
                        let points = line.map(\.coordinate)
                        return MKPolyline(coordinates: points, count: points.count)
                    }
                    overlay = LineGroup(lines, role: layer.role, widthMeters: layer.widthM)
                default:
                    overlay = PolygonGroup(layer.polygons.map(\.shape), role: layer.role)
                }
                let level = Self.level(for: layer.role)
                let position = layers[..<index].filter { Self.level(for: $0.role) == level }.count
                map.insertOverlay(overlay, at: position, level: level)
                overlays[layer.id] = overlay
            }
        }

        func draw(_ records: [CampusMapMark], on map: MKMapView) {
            let wanted = Set(records.map(\.id))
            let removed = marks.filter { !wanted.contains($0.key) }.map(\.value)
            if !removed.isEmpty { map.removeAnnotations(removed) }
            removed.forEach { marks.removeValue(forKey: $0.id) }
            var added: [FloorMark] = []
            for record in records {
                let mark = FloorMark(record)
                if let old = marks[mark.id], old.record == record { continue }
                if let old = marks[mark.id] { map.removeAnnotation(old) }
                marks[mark.id] = mark
                added.append(mark)
            }
            if !added.isEmpty { map.addAnnotations(added) }
        }

        /// Theme changes repaint existing native objects; they do not rebuild
        /// the scene or replace annotations while a text field is active.
        func prepareAppearance(_ value: String, on map: MKMapView) {
            guard value != appearance else { return }
            appearance = value
            for overlay in overlays.values {
                if let group = overlay as? PolygonGroup,
                    let renderer = map.renderer(for: group) as? MKMultiPolygonRenderer {
                    Self.style(renderer, role: group.role, traits: map.traitCollection)
                    renderer.setNeedsDisplay()
                } else if let group = overlay as? LineGroup,
                    let renderer = map.renderer(for: group) as? MetricLineRenderer {
                    renderer.color = Self.color(for: group.role, traits: map.traitCollection).cgColor
                    renderer.setNeedsDisplay()
                }
            }
            for mark in marks.values {
                (map.view(for: mark) as? FloorMarkView)?.show(mark, traits: map.traitCollection)
            }
            stackView?.restyle()
        }

        func mapView(_ map: MKMapView, rendererFor overlay: MKOverlay) -> MKOverlayRenderer {
            if let group = overlay as? LineGroup {
                return MetricLineRenderer(
                    group: group, color: Self.color(for: group.role, traits: map.traitCollection))
            }
            if let group = overlay as? PolygonGroup {
                let renderer = MKMultiPolygonRenderer(multiPolygon: group)
                Self.style(renderer, role: group.role, traits: map.traitCollection)
                return renderer
            }
            return MKOverlayRenderer(overlay: overlay)
        }

        func mapView(_ map: MKMapView, viewFor annotation: MKAnnotation) -> MKAnnotationView? {
            guard let mark = annotation as? FloorMark else { return nil }
            let view = map.dequeueReusableAnnotationView(withIdentifier: FloorMarkView.reuse, for: mark)
            (view as? FloorMarkView)?.show(mark, traits: map.traitCollection)
            return view
        }

        private static func level(for role: CampusMapLayerRole) -> MKOverlayLevel {
            switch role {
            case .footprint, .focusedFootprint: .aboveRoads
            default: .aboveLabels
            }
        }

        private static func color(for role: CampusMapLayerRole, traits: UITraitCollection) -> UIColor {
            let color: Color = switch role {
            case .detailLine: Theme.muted
            case .wallLine: Theme.ink
            default: .clear
            }
            return UIColor(color).resolvedColor(with: traits)
        }

        private static func style(
            _ renderer: MKMultiPolygonRenderer,
            role: CampusMapLayerRole,
            traits: UITraitCollection
        ) {
            let color = { (role: Color) in UIColor(role).resolvedColor(with: traits) }
            renderer.fillColor = .clear
            renderer.strokeColor = .clear
            renderer.lineWidth = 0
            switch role {
            case .footprint:
                renderer.fillColor = color(Theme.surface2).withAlphaComponent(0.8)
                renderer.strokeColor = color(Theme.line)
                renderer.lineWidth = 1
            case .focusedFootprint:
                renderer.fillColor = color(Theme.surface)
                renderer.strokeColor = color(Theme.accent)
                renderer.lineWidth = 1.5
            case .levelOutline:
                renderer.fillColor = color(Theme.surface)
                renderer.strokeColor = color(Theme.line)
                renderer.lineWidth = 1
            case .classroom:
                renderer.fillColor = color(Theme.surface2)
            case .highlight:
                renderer.fillColor = color(Theme.accent).withAlphaComponent(0.35)
                renderer.strokeColor = color(Theme.accent)
                renderer.lineWidth = 2
            case .detailLine, .wallLine:
                break
            }
        }
    }
}

/// A passive, app-owned contact stream for pan, pinch and rotation alike.
/// It neither delays nor cancels touches and cannot block native recognizers.
final class MapContactRecognizer: UIGestureRecognizer {
    private var fingers: Set<UITouch> = []
    var fingersDown: Bool { !fingers.isEmpty }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent) {
        super.touchesBegan(touches, with: event)
        fingers.formUnion(touches)
        state = state == .possible ? .began : .changed
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent) {
        super.touchesMoved(touches, with: event)
        state = .changed
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent) {
        super.touchesEnded(touches, with: event)
        fingers.subtract(touches)
        state = fingers.isEmpty ? .ended : .changed
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent) {
        super.touchesCancelled(touches, with: event)
        fingers.subtract(touches)
        state = fingers.isEmpty ? .cancelled : .changed
    }

    override func reset() {
        super.reset()
        fingers.removeAll()
    }

    override func canPrevent(_ preventedGestureRecognizer: UIGestureRecognizer) -> Bool { false }
    override func canBePrevented(by preventingGestureRecognizer: UIGestureRecognizer) -> Bool { false }
}

/// `setVisibleMapRect` needs a real size, which a new map does not have yet;
/// the compass follows the visible part on every layout.
final class MeasuredMapView: MKMapView {
    var laidOut: (() -> Void)?

    override func layoutSubviews() {
        super.layoutSubviews()
        laidOut?()
    }
}

private final class PolygonGroup: MKMultiPolygon {
    private(set) var role = CampusMapLayerRole.footprint

    convenience init(_ polygons: [MKPolygon], role: CampusMapLayerRole) {
        self.init(polygons)
        self.role = role
    }
}

private final class LineGroup: MKMultiPolyline {
    private(set) var role = CampusMapLayerRole.wallLine
    private(set) var widthMeters = 0.0

    convenience init(_ lines: [MKPolyline], role: CampusMapLayerRole, widthMeters: Double) {
        self.init(lines)
        self.role = role
        self.widthMeters = widthMeters
    }
}

/// Lines retain their physical width as the map zooms, with a hairline floor.
private final class MetricLineRenderer: MKOverlayRenderer {
    var color: CGColor
    private let linePath = CGMutablePath()
    private let width: CGFloat

    init(group: LineGroup, color: UIColor) {
        self.color = color.cgColor
        width = group.widthMeters * MKMapPointsPerMeterAtLatitude(group.coordinate.latitude)
        super.init(overlay: group)
        for line in group.polylines where line.pointCount > 1 {
            let points = line.points()
            linePath.move(to: point(for: points[0]))
            for index in 1..<line.pointCount { linePath.addLine(to: point(for: points[index])) }
        }
    }

    override func draw(_ mapRect: MKMapRect, zoomScale: MKZoomScale, in context: CGContext) {
        context.addPath(linePath)
        context.setStrokeColor(color)
        context.setLineCap(.round)
        context.setLineJoin(.round)
        context.setLineWidth(max(width, 0.5 / zoomScale))
        context.strokePath()
    }
}

private final class FloorMark: NSObject, MKAnnotation {
    let id: String
    let record: CampusMapMark
    let coordinate: CLLocationCoordinate2D

    init(_ mark: CampusMapMark) {
        id = mark.id
        record = mark
        coordinate = mark.point.coordinate
    }
}

/// MapKit drops colliding marks as the map zooms out, building names first;
/// highlighted rooms and entrances remain visible. A room mark selects its room; an entrance mark
/// is 44 pt to touch around its 28 pt badge.
private final class FloorMarkView: MKAnnotationView {
    static let reuse = "floor-mark"
    private static let iconSide: CGFloat = 28
    /// The smallest a map icon gets (slotledger `approach/campus-map-icons`).
    private static let passageSide: CGFloat = 24
    private static let touchSide: CGFloat = 44
    private let label = UILabel()
    private let badge = UIView()
    private let icon = UIImageView()

    override init(annotation: MKAnnotation?, reuseIdentifier: String?) {
        super.init(annotation: annotation, reuseIdentifier: reuseIdentifier)
        collisionMode = .rectangle
        canShowCallout = false
        isAccessibilityElement = true
        addSubview(label)
        addSubview(badge)
        badge.addSubview(icon)
        badge.isUserInteractionEnabled = false
        icon.contentMode = .scaleAspectFit
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { nil }

    func show(_ mark: FloorMark, traits: UITraitCollection) {
        annotation = mark
        let color = { (role: Color) in UIColor(role).resolvedColor(with: traits) }
        switch mark.record.kind {
        case let .room(_, name, highlighted):
            badge.isHidden = true
            label.isHidden = false
            label.text = name
            label.font = .systemFont(ofSize: highlighted ? 13 : 10, weight: highlighted ? .bold : .medium)
            label.textColor = color(highlighted ? Theme.accent : Theme.muted)
            label.layer.shadowOpacity = 0
            label.sizeToFit()
            frame.size = label.bounds.size
            label.frame.origin = .zero
            displayPriority = highlighted ? .required : .defaultLow
            zPriority = highlighted ? .max : .defaultUnselected
            accessibilityLabel = name
            accessibilityTraits = highlighted ? [.button, .selected] : .button
        case let .building(_, name):
            // Under the room labels: a building's own floor names it best.
            badge.isHidden = true
            label.isHidden = false
            label.text = name
            label.font = .systemFont(ofSize: 11, weight: .semibold)
            label.textColor = color(Theme.ink)
            // A halo in the surface colour keeps the name readable on the base map.
            label.layer.shadowColor = color(Theme.surface).cgColor
            label.layer.shadowOffset = .zero
            label.layer.shadowRadius = 2
            label.layer.shadowOpacity = 1
            label.sizeToFit()
            frame.size = label.bounds.size
            label.frame.origin = .zero
            displayPriority = MKFeatureDisplayPriority(rawValue: 200)
            zPriority = .defaultUnselected
            accessibilityLabel = name
            accessibilityTraits = .button
        case let .amenity(key, tone):
            // A white glyph on a badge of its class's colour; stairs and
            // elevators are smaller and give way first, under room labels.
            let passage = tone == .passage
            showIcon(
                key, side: passage ? Self.passageSide : Self.iconSide, tint: .white,
                fill: Self.fill(for: tone).resolvedColor(with: traits), traits: traits, badge: true)
            displayPriority = Self.priority(for: tone)
            zPriority = passage ? .min : .defaultUnselected
            accessibilityLabel = t("campusMap.amenity.\(key)")
            accessibilityTraits = .image
        case let .entrance(_, name, selected):
            showIcon(
                "entrance", side: Self.touchSide,
                tint: color(selected ? Theme.accentInk : Theme.accent),
                fill: color(selected ? Theme.accent : Theme.surface), traits: traits)
            displayPriority = .required
            zPriority = selected ? .max : .defaultSelected
            accessibilityLabel = name ?? t("campusMap.entrance")
            accessibilityTraits = selected ? [.button, .selected] : .button
        }
    }

    /// A round mark (or, with `badge`, a rounded square) of at most
    /// 28 pt centred in a frame of `side`, the area MapKit hit-tests.
    private func showIcon(
        _ name: String, side: CGFloat, tint: UIColor, fill: UIColor, traits: UITraitCollection, badge square: Bool = false
    ) {
        label.isHidden = true
        badge.isHidden = false
        frame.size = CGSize(width: side, height: side)
        let mark = min(side, Self.iconSide)
        let inset = (side - mark) / 2
        badge.frame = bounds.insetBy(dx: inset, dy: inset)
        badge.backgroundColor = fill
        badge.layer.cornerRadius = square ? mark * 0.26 : mark / 2
        badge.layer.cornerCurve = .continuous
        // A hairline in the surface colour keeps a coloured badge apart from
        // the floor lines under it.
        badge.layer.borderWidth = square ? 1.5 : 1
        badge.layer.borderColor = UIColor(square ? Theme.surface : Theme.line).resolvedColor(with: traits).cgColor
        icon.image = UIImage(named: "CampusMap/\(name)")?.withRenderingMode(.alwaysTemplate)
        icon.tintColor = tint
        icon.frame = badge.bounds.insetBy(dx: mark * 0.18, dy: mark * 0.18)
    }

    /// The class colours of slotledger `approach/campus-map-icons`, the
    /// system's so they hold in dark mode and match Apple Maps' wayfinding.
    private static func fill(for tone: CampusMapAmenityTone) -> UIColor {
        switch tone {
        case .aed: .systemRed
        case .restroom: .systemBlue
        case .stay: .systemGreen
        case .passage: .systemGray
        }
    }

    /// Core lists tones most important first; MapKit keeps the higher one.
    private static func priority(for tone: CampusMapAmenityTone) -> MKFeatureDisplayPriority {
        switch tone {
        case .aed: .defaultHigh
        case .restroom: MKFeatureDisplayPriority(rawValue: 700)
        case .stay: MKFeatureDisplayPriority(rawValue: 650)
        // Under the room labels (250), over the building names (200).
        case .passage: MKFeatureDisplayPriority(rawValue: 240)
        }
    }
}

private extension CampusMapPolygon {
    var shape: MKPolygon {
        let rings = rings.map { $0.map(\.coordinate) }
        let holes = rings.dropFirst().map { MKPolygon(coordinates: $0, count: $0.count) }
        let outer = rings.first ?? []
        return MKPolygon(coordinates: outer, count: outer.count, interiorPolygons: Array(holes))
    }
}

private extension CampusMapPoint {
    var coordinate: CLLocationCoordinate2D {
        CLLocationCoordinate2D(latitude: latitude, longitude: longitude)
    }
}

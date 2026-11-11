import IsctCore
import MapKit
import SwiftUI

/// Where the page's own controls sit, in window coordinates: the close
/// button's bottom edge (above the sheet), the sheet's top edge, the leading
/// panel's trailing edge and the control rail's leading edge. The rail stays
/// reserved when a floor picker is not shown.
struct CampusMapChrome: Equatable {
    static let controlSide: CGFloat = 44
    static let controlGap: CGFloat = 8
    static let pageInset: CGFloat = 12
    static let railHeaderHeight = 2 * (controlSide + controlGap)

    var top: CGFloat = 0
    var bottom: CGFloat = .greatestFiniteMagnitude
    var leading: CGFloat = 0
    var trailing: CGFloat?
    /// The request a completed panel layout pass measured these edges for.
    var layout: CampusMapLayoutRequest?
}

/// The panel's heights: the sheet's detents, or the leading panel's.
enum CampusMapPanelHeight: Hashable {
    case small, medium, large
}

/// What the map waits on before an explicit framing: the framing itself and
/// the panel state it is shown beside.
struct CampusMapLayoutRequest: Equatable {
    let framing: UInt64
    let searching: Bool
    let height: CampusMapPanelHeight
    /// The leading panel rather than the sheet.
    let wide: Bool

    /// A large sheet covers the map.
    var hidesControls: Bool { !wide && height == .large }
    var canFrame: Bool { !searching && !hidesControls }
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
            // A detent change lays out at its target frame.
            DispatchQueue.main.async { [weak self] in
                guard let self, window != nil, self.request == request else { return }
                onLayout?(request, convert(bounds, to: nil))
            }
        }
    }
}

/// Apple's base map with Core's scene drawn over it by role. Core decides
/// every layer and mark; this view only batches and paints them, and reports
/// the part of the map the controls leave visible.
struct FloorMapView: UIViewRepresentable {
    let scene: CampusMapScene?
    let chrome: CampusMapChrome
    let layoutRequest: CampusMapLayoutRequest
    let showsPosition: Bool
    /// Rises when the person asked for their position and may now see it.
    let follows: Int
    /// Where the camera settled, with the last framing key applied.
    let onCamera: (CampusMapCamera, UInt64) -> Void
    let onTap: (CampusMapPoint, String?) -> Void
    let onSelect: (CampusMapPlace) -> Void
    /// The person turned on following their position.
    let onLocate: () -> Void

    /// Room between the visible part's edges and what a framing shows.
    private static let framingMargin: CGFloat = 16

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeUIView(context: Context) -> MeasuredMapView {
        let map = MeasuredMapView()
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
        // A double tap zooms, which MapKit does; this recognizer only makes
        // a single tap wait for it, so a zoom selects nothing.
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
        let tracking = MKUserTrackingButton(mapView: map)
        tracking.backgroundColor = UIColor(Theme.surface)
        tracking.layer.cornerRadius = 8
        tracking.clipsToBounds = true
        // Gate only permission on an actual tap (including VoiceOver).
        // After authorization the native button receives touches directly
        // and owns the follow / heading / off cycle.
        let trackingHost = UIButton(type: .custom)
        trackingHost.addTarget(coordinator, action: #selector(Coordinator.locateTapped), for: .touchUpInside)
        trackingHost.addSubview(tracking)
        tracking.frame = CGRect(x: 0, y: 0, width: CampusMapChrome.controlSide, height: CampusMapChrome.controlSide)
        map.addSubview(trackingHost)
        coordinator.trackingHost = trackingHost
        coordinator.tracking = tracking
        let compass = MKCompassButton(mapView: map)
        compass.compassVisibility = .adaptive
        map.addSubview(compass)
        coordinator.compass = compass
        map.laidOut = { [weak coordinator, weak map] in
            guard let coordinator, let map else { return }
            coordinator.layOut(map)
            coordinator.applyFraming(to: map)
        }
        return map
    }

    func updateUIView(_ map: MeasuredMapView, context: Context) {
        let coordinator = context.coordinator
        coordinator.onCamera = onCamera
        coordinator.onTap = onTap
        coordinator.onSelect = onSelect
        coordinator.onLocate = onLocate
        coordinator.animatesCamera = !context.environment.accessibilityReduceMotion
        coordinator.chrome = chrome
        coordinator.layoutRequest = layoutRequest
        coordinator.layOut(map)
        coordinator.trackingHost?.isAccessibilityElement = !showsPosition
        coordinator.trackingHost?.accessibilityLabel = t("campusMap.position")
        coordinator.tracking?.isUserInteractionEnabled = showsPosition
        coordinator.tracking?.accessibilityElementsHidden = !showsPosition
        if map.showsUserLocation != showsPosition { map.showsUserLocation = showsPosition }
        if follows != coordinator.follows {
            coordinator.follows = follows
            map.setUserTrackingMode(.follow, animated: coordinator.animatesCamera)
        }
        let appearance = "\(context.environment.colorScheme)|\(context.environment.isctTheme)"
        coordinator.prepareAppearance(appearance, on: map)
        if coordinator.drawnPage != scene?.page || coordinator.drawnRevision != scene?.revision {
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
        coordinator.applyFraming(to: map)
    }

    final class Coordinator: NSObject, MKMapViewDelegate, UIGestureRecognizerDelegate {
        var onCamera: (CampusMapCamera, UInt64) -> Void = { _, _ in }
        var onTap: (CampusMapPoint, String?) -> Void = { _, _ in }
        var onSelect: (CampusMapPlace) -> Void = { _ in }
        var onLocate: () -> Void = {}
        var animatesCamera = true
        var chrome = CampusMapChrome()
        var layoutRequest: CampusMapLayoutRequest?
        var follows = 0
        var levelId: String?
        var drawnPage: UInt64?
        var drawnRevision: UInt64?
        var appliedFraming: UInt64 = 0
        var pendingFraming: CampusMapFraming?
        weak var trackingHost: UIButton?
        weak var tracking: MKUserTrackingButton?
        weak var compass: MKCompassButton?
        weak var contact: MapContactRecognizer?
        private var framed = false
        /// Serialize native animations; a newer Core command replaces the
        /// pending command while the current move settles. Reports keep the
        /// applied key, so Core rejects a superseded landing.
        private var activeFraming: UInt64?
        private var announced = false
        private var appearance: String?
        private var overlays: [String: MKOverlay] = [:]
        private var marks: [String: FloorMark] = [:]

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

        /// Native controls occupy the trailing rail: standing on the sheet's
        /// top edge, or at the top beside the leading panel, and hidden with
        /// the rail while the sheet covers the map. Layout alone never
        /// changes the camera.
        func layOut(_ map: MKMapView) {
            guard map.bounds.width > 0, map.bounds.height > 0 else { return }
            let visible = visible(in: map)
            let side = CampusMapChrome.controlSide
            let gap = CampusMapChrome.controlGap
            let x = chrome.trailing == nil
                ? map.bounds.maxX - map.safeAreaInsets.right - side - CampusMapChrome.pageInset
                : visible.maxX
            let compassY: CGFloat
            let trackingY: CGFloat
            if layoutRequest?.wide == true {
                compassY = visible.minY + gap
                trackingY = compassY + side + gap
            } else {
                trackingY = visible.maxY - gap - side
                compassY = trackingY - gap - side
            }
            compass?.frame = CGRect(x: x, y: compassY, width: side, height: side)
            trackingHost?.frame = CGRect(x: x, y: trackingY, width: side, height: side)
            let hidden = layoutRequest?.hidesControls ?? false
            compass?.isHidden = hidden
            trackingHost?.isHidden = hidden
        }

        // MARK: Framing

        func applyFraming(to map: MKMapView) {
            guard let layoutRequest, layoutRequest.canFrame, chrome.layout == layoutRequest,
                let framing = pendingFraming, framing.key == layoutRequest.framing,
                activeFraming == nil, contact?.fingersDown != true, chrome.trailing != nil,
                map.bounds.width > 0, map.bounds.height > 0
            else { return }
            pendingFraming = nil
            appliedFraming = framing.key
            frame(framing, in: visible(in: map), on: map, animated: framed && animatesCamera)
            framed = true
        }

        private func frame(_ framing: CampusMapFraming, in visible: CGRect, on map: MKMapView, animated: Bool) {
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
            activeFraming = framing.key
            announced = false
            map.setVisibleMapRect(rect, edgePadding: padding, animated: animated)
            // A no-op framing need not produce region callbacks. Give MapKit
            // the current update before acknowledging it, rather than assuming
            // callbacks must occur synchronously inside setVisibleMapRect.
            DispatchQueue.main.async { [weak self, weak map] in
                guard let self, let map, activeFraming == framing.key, !announced else { return }
                finishFraming(on: map)
            }
        }

        // MARK: Camera and taps

        func mapView(_ map: MKMapView, regionWillChangeAnimated animated: Bool) {
            if contact?.fingersDown == true {
                activeFraming = nil
            } else if activeFraming != nil {
                announced = true
            }
        }

        func mapView(_ map: MKMapView, regionDidChangeAnimated animated: Bool) {
            finishFraming(on: map, byUser: activeFraming == nil || contact?.fingersDown == true)
        }

        @objc func contactChanged(_ recognizer: MapContactRecognizer) {
            if recognizer.fingersDown {
                // Latch the interruption through deceleration after release.
                activeFraming = nil
            } else if let map = recognizer.view as? MKMapView {
                DispatchQueue.main.async { [weak self, weak map] in
                    guard let self, let map else { return }
                    applyFraming(to: map)
                }
            }
        }

        private func finishFraming(on map: MKMapView, byUser: Bool = false) {
            activeFraming = nil
            report(map, byUser: byUser)
            // Avoid starting another native move from inside its delegate.
            DispatchQueue.main.async { [weak self, weak map] in
                guard let self, let map else { return }
                applyFraming(to: map)
            }
        }

        private func report(_ map: MKMapView, byUser: Bool) {
            let visible = visible(in: map)
            let at = { (x: CGFloat, y: CGFloat) -> CampusMapPoint in
                let coordinate = map.convert(CGPoint(x: x, y: y), toCoordinateFrom: map)
                return CampusMapPoint(latitude: coordinate.latitude, longitude: coordinate.longitude)
            }
            let camera = CampusMapCamera(
                topLeft: at(visible.minX, visible.minY),
                topRight: at(visible.maxX, visible.minY),
                bottomRight: at(visible.maxX, visible.maxY),
                bottomLeft: at(visible.minX, visible.maxY),
                widthPoints: Double(visible.width),
                heightPoints: Double(visible.height),
                byUser: byUser)
            DispatchQueue.main.async { [onCamera, appliedFraming] in
                onCamera(camera, appliedFraming)
            }
        }

        @objc func locateTapped() { onLocate() }

        func gestureRecognizer(
            _ gestureRecognizer: UIGestureRecognizer,
            shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer
        ) -> Bool { true }

        func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
            if gestureRecognizer is MapContactRecognizer { return true }
            var hit = touch.view
            while let view = hit, view !== gestureRecognizer.view {
                if view is UIControl || view is MKAnnotationView || view is MKUserTrackingButton || view is MKCompassButton {
                    return false
                }
                hit = view.superview
            }
            return true
        }

        @objc func doubleTapped(_ recognizer: UITapGestureRecognizer) {}

        @objc func tapped(_ recognizer: UITapGestureRecognizer) {
            guard recognizer.state == .ended, let map = recognizer.view as? MKMapView else { return }
            let location = recognizer.location(in: map)
            let coordinate = map.convert(location, toCoordinateFrom: map)
            onTap(
                CampusMapPoint(latitude: coordinate.latitude, longitude: coordinate.longitude),
                levelId)
        }

        /// A tap on a mark, or VoiceOver activating one.
        func mapView(_ map: MKMapView, didSelect view: MKAnnotationView) {
            if let mark = view.annotation as? FloorMark {
                switch mark.record.kind {
                case let .entrance(id, _, _): onSelect(.entrance(id: id))
                case let .room(unitId, _, _): onSelect(.unit(id: unitId))
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
/// the position button follows the visible part on every layout.
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

/// MapKit drops colliding marks as the map zooms out; highlighted rooms and
/// entrances remain visible. A room mark selects its room; an entrance mark
/// is 44 pt to touch around its 28 pt badge.
private final class FloorMarkView: MKAnnotationView {
    static let reuse = "floor-mark"
    private static let iconSide: CGFloat = 28
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
            label.sizeToFit()
            frame.size = label.bounds.size
            label.frame.origin = .zero
            displayPriority = highlighted ? .required : .defaultLow
            zPriority = highlighted ? .max : .defaultUnselected
            accessibilityLabel = name
            accessibilityTraits = highlighted ? [.button, .selected] : .button
        case let .amenity(key):
            showIcon(key, side: Self.iconSide, tint: color(Theme.ink), fill: color(Theme.surface), traits: traits)
            displayPriority = .defaultHigh
            zPriority = .defaultUnselected
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

    /// A 28 pt badge centred in a frame of `side`, the area MapKit hit-tests.
    private func showIcon(_ name: String, side: CGFloat, tint: UIColor, fill: UIColor, traits: UITraitCollection) {
        label.isHidden = true
        badge.isHidden = false
        frame.size = CGSize(width: side, height: side)
        let inset = (side - Self.iconSide) / 2
        badge.frame = bounds.insetBy(dx: inset, dy: inset)
        badge.backgroundColor = fill
        badge.layer.cornerRadius = Self.iconSide / 2
        badge.layer.borderWidth = 1
        badge.layer.borderColor = UIColor(Theme.line).resolvedColor(with: traits).cgColor
        icon.image = UIImage(named: "CampusMap/\(name)")?.withRenderingMode(.alwaysTemplate)
        icon.tintColor = tint
        icon.frame = badge.bounds.insetBy(dx: 5, dy: 5)
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

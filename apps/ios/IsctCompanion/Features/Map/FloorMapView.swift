import IsctCore
import MapKit
import SwiftUI

/// Where the page's own controls sit, in window coordinates: the search
/// bar's bottom edge, the bottom panel's top edge and the level picker's
/// leading edge (`nil` when it is hidden). The map is visible between them.
struct CampusMapChrome: Equatable {
    var top: CGFloat = 0
    var bottom: CGFloat = .greatestFiniteMagnitude
    var trailing: CGFloat?
}

/// Apple's base map with Core's scene drawn over it by role. Core decides
/// every layer and mark; this view only batches and paints them, and reports
/// the part of the map the controls leave visible.
struct FloorMapView: UIViewRepresentable {
    let scene: CampusMapScene?
    let chrome: CampusMapChrome
    /// Rises each time the person changes the page's controls (folds the
    /// card, opens or leaves the search, closes the card): a change of the
    /// visible part that comes from them never re-frames.
    let chromeEdits: Int
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
        map.showsCompass = true
        // The margins are the page's controls, not the window's safe area.
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
        map.addGestureRecognizer(double)
        map.addGestureRecognizer(tap)
        let tracking = MKUserTrackingButton(mapView: map)
        tracking.backgroundColor = UIColor(Theme.surface)
        tracking.layer.cornerRadius = 8
        tracking.clipsToBounds = true
        map.addSubview(tracking)
        coordinator.tracking = tracking
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
        coordinator.chromeEdits = chromeEdits
        coordinator.layOut(map)
        if map.showsUserLocation != showsPosition { map.showsUserLocation = showsPosition }
        if follows != coordinator.follows {
            coordinator.follows = follows
            map.setUserTrackingMode(.follow, animated: coordinator.animatesCamera)
        }
        let appearance = "\(context.environment.colorScheme)|\(context.environment.isctTheme)"
        coordinator.prepareAppearance(appearance, on: map)
        coordinator.draw(scene?.layers ?? [], on: map)
        coordinator.draw(scene?.marks ?? [], on: map)
        coordinator.levelId = scene?.levelId
        coordinator.sceneFraming = scene?.framing?.key ?? 0
        if let framing = scene?.framing,
            framing.key > coordinator.appliedFraming,
            framing.key != coordinator.pendingFraming?.key
        {
            coordinator.pendingFraming = framing
            coordinator.applyFraming(to: map)
        } else {
            coordinator.correctFraming(on: map)
        }
    }

    final class Coordinator: NSObject, MKMapViewDelegate, UIGestureRecognizerDelegate {
        var onCamera: (CampusMapCamera, UInt64) -> Void = { _, _ in }
        var onTap: (CampusMapPoint, String?) -> Void = { _, _ in }
        var onSelect: (CampusMapPlace) -> Void = { _ in }
        var onLocate: () -> Void = {}
        var animatesCamera = true
        var chrome = CampusMapChrome()
        var chromeEdits = 0
        var follows = 0
        var levelId: String?
        /// The framing key of the scene drawn.
        var sceneFraming: UInt64 = 0
        var appliedFraming: UInt64 = 0
        var pendingFraming: CampusMapFraming?
        weak var tracking: MKUserTrackingButton?
        private var framed = false
        /// Region changes the app started and MapKit has not finished: each
        /// `setVisibleMapRect` that announces a move adds one, each settled
        /// region takes one. While any is left a change is the app's; one
        /// interrupted by the next framing still settles as the app's.
        private var appMoves = 0
        private var insideCall = false
        private var announced = false
        /// The last framing, until the person moves or taps the map: if the
        /// layout it was framed in settles to another visible part (the card
        /// measured after the framing, the level picker shown on landing) it
        /// is framed once more. Changes the person makes to the page's
        /// controls never re-frame.
        private var correction: (framing: CampusMapFraming, edits: Int, visible: CGRect)?
        private var appearance: String?
        private var overlays: [String: MKOverlay] = [:]
        private var marks: [String: FloorMark] = [:]

        // MARK: Visible part

        /// The map less the page's controls, in the map's own points.
        func visible(in map: MKMapView) -> CGRect {
            let bounds = map.bounds
            let inMap = { (point: CGPoint) in map.window == nil ? point : map.convert(point, from: nil) }
            let top = max(inMap(CGPoint(x: 0, y: chrome.top)).y, bounds.minY)
            let bottom = min(inMap(CGPoint(x: 0, y: min(chrome.bottom, 1e6))).y, bounds.maxY)
            let trailing = chrome.trailing.map { min(inMap(CGPoint(x: $0, y: 0)).x, bounds.maxX) } ?? bounds.maxX
            let leading = bounds.minX + map.safeAreaInsets.left
            let right = min(trailing, bounds.maxX - map.safeAreaInsets.right)
            return CGRect(
                x: leading, y: top,
                width: max(right - leading, 1), height: max(bottom - top, 1))
        }

        /// Keeps Apple's logo, the legal link and the compass in the visible
        /// part, and the position button at its bottom trailing corner.
        func layOut(_ map: MKMapView) {
            guard map.bounds.width > 0, map.bounds.height > 0 else { return }
            let visible = visible(in: map)
            let margins = UIEdgeInsets(
                top: visible.minY, left: visible.minX + 8,
                bottom: map.bounds.maxY - visible.maxY, right: map.bounds.maxX - visible.maxX + 8)
            if map.layoutMargins != margins { map.layoutMargins = margins }
            if let tracking {
                let side: CGFloat = 44
                tracking.frame = CGRect(
                    x: visible.maxX - side - 8, y: visible.maxY - side - 8, width: side, height: side)
            }
        }

        // MARK: Framing

        func applyFraming(to map: MKMapView) {
            guard let framing = pendingFraming, map.bounds.width > 0, map.bounds.height > 0 else { return }
            pendingFraming = nil
            appliedFraming = framing.key
            let visible = visible(in: map)
            frame(framing, in: visible, on: map, animated: framed && animatesCamera)
            correction = (framing, chromeEdits, visible)
            framed = true
        }

        /// Once, when the layout a framing was made in has settled to another
        /// visible part without the person moving or tapping the map or
        /// changing the page's controls in between, and no newer framing.
        func correctFraming(on map: MKMapView) {
            guard let pending = correction, pending.framing.key == sceneFraming, pending.edits == chromeEdits
            else {
                correction = nil
                return
            }
            let visible = visible(in: map)
            guard visible != pending.visible else { return }
            correction = nil
            frame(pending.framing, in: visible, on: map, animated: animatesCamera)
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
            insideCall = true
            announced = false
            map.setVisibleMapRect(rect, edgePadding: padding, animated: animated)
            insideCall = false
            // MapKit announces a move from inside the call; none means the
            // camera is already there, and the framing has landed.
            if !announced && appMoves == 0 { report(map, byUser: false) }
        }

        // MARK: Camera and taps

        func mapView(_ map: MKMapView, regionWillChangeAnimated animated: Bool) {
            if insideCall {
                announced = true
                appMoves += 1
            }
        }

        func mapView(_ map: MKMapView, regionDidChangeAnimated animated: Bool) {
            if appMoves > 0 {
                appMoves -= 1
                // A move interrupted by a newer one: the newer one reports.
                if appMoves == 0 { report(map, byUser: false) }
                return
            }
            correction = nil
            report(map, byUser: true)
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

        func mapView(_ map: MKMapView, didChange mode: MKUserTrackingMode, animated: Bool) {
            if mode != .none { onLocate() }
        }

        func gestureRecognizer(
            _ gestureRecognizer: UIGestureRecognizer,
            shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer
        ) -> Bool { true }

        @objc func doubleTapped(_ recognizer: UITapGestureRecognizer) {}

        @objc func tapped(_ recognizer: UITapGestureRecognizer) {
            guard recognizer.state == .ended, let map = recognizer.view as? MKMapView else { return }
            let location = recognizer.location(in: map)
            var hit = map.hitTest(location, with: nil)
            while let view = hit {
                // A mark is picked through its selection; the position button
                // is its own control.
                if view is MKAnnotationView || view is MKUserTrackingButton { return }
                hit = view.superview
            }
            correction = nil
            let coordinate = map.convert(location, toCoordinateFrom: map)
            onTap(
                CampusMapPoint(latitude: coordinate.latitude, longitude: coordinate.longitude),
                levelId)
        }

        /// A tap on a mark, or VoiceOver activating one.
        func mapView(_ map: MKMapView, didSelect view: MKAnnotationView) {
            correction = nil
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
            map.removeOverlays(Array(removed.values))
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
            map.removeAnnotations(removed)
            removed.forEach { marks.removeValue(forKey: $0.id) }
            var added: [FloorMark] = []
            for record in records {
                let mark = FloorMark(record)
                if let old = marks[mark.id], old.record == record { continue }
                if let old = marks[mark.id] { map.removeAnnotation(old) }
                marks[mark.id] = mark
                added.append(mark)
            }
            map.addAnnotations(added)
        }

        /// Theme colours are layer content too; a rare theme change redraws
        /// them without complicating every renderer with mutable styling.
        func prepareAppearance(_ value: String, on map: MKMapView) {
            guard value != appearance else { return }
            appearance = value
            map.removeOverlays(Array(overlays.values))
            map.removeAnnotations(Array(marks.values))
            overlays.removeAll()
            marks.removeAll()
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
    private let color: CGColor
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

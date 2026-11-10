import IsctCore
import MapKit
import SwiftUI

/// Apple's base map with Core's scene drawn over it by role. Core decides
/// every layer and mark; this view only batches and paints them.
struct FloorMapView: UIViewRepresentable {
    let scene: CampusMapScene?
    /// What the card and the level picker cover, kept clear when framing.
    let insets: EdgeInsets
    let showsPosition: Bool
    /// Where the camera settled, with the last framing key applied.
    let onCamera: (CampusMapCamera, UInt64) -> Void
    let onTap: (CampusMapPoint, String?) -> Void
    let onEntrance: (String) -> Void

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeUIView(context: Context) -> MeasuredMapView {
        let map = MeasuredMapView()
        let configuration = MKStandardMapConfiguration(elevationStyle: .flat, emphasisStyle: .muted)
        configuration.pointOfInterestFilter = .excludingAll
        map.preferredConfiguration = configuration
        map.isPitchEnabled = false
        map.showsCompass = true
        map.delegate = context.coordinator
        map.register(FloorMarkView.self, forAnnotationViewWithReuseIdentifier: FloorMarkView.reuse)
        let tap = UITapGestureRecognizer(target: context.coordinator, action: #selector(Coordinator.tapped(_:)))
        tap.cancelsTouchesInView = false
        tap.delegate = context.coordinator
        // A double tap zooms; it selects nothing.
        for other in map.subviews.first?.gestureRecognizers ?? [] {
            if let zoom = other as? UITapGestureRecognizer, zoom.numberOfTapsRequired == 2 {
                tap.require(toFail: zoom)
            }
        }
        map.addGestureRecognizer(tap)
        map.laidOut = { [weak coordinator = context.coordinator, weak map] in
            if let map { coordinator?.applyFraming(to: map) }
        }
        return map
    }

    func updateUIView(_ map: MeasuredMapView, context: Context) {
        let coordinator = context.coordinator
        coordinator.onCamera = onCamera
        coordinator.onTap = onTap
        coordinator.onEntrance = onEntrance
        coordinator.animatesCamera = !context.environment.accessibilityReduceMotion
        coordinator.insets = UIEdgeInsets(
            top: insets.top, left: insets.leading, bottom: insets.bottom, right: insets.trailing)
        if map.showsUserLocation != showsPosition { map.showsUserLocation = showsPosition }
        let appearance = "\(context.environment.colorScheme)|\(context.environment.isctTheme)"
        coordinator.prepareAppearance(appearance, on: map)
        coordinator.draw(scene?.layers ?? [], on: map)
        coordinator.draw(scene?.marks ?? [], on: map)
        coordinator.levelId = scene?.levelId
        if let framing = scene?.framing,
            framing.key > coordinator.appliedFraming,
            framing.key != coordinator.pendingFraming?.key
        {
            coordinator.pendingFraming = framing
            coordinator.applyFraming(to: map)
        }
    }

    final class Coordinator: NSObject, MKMapViewDelegate, UIGestureRecognizerDelegate {
        var onCamera: (CampusMapCamera, UInt64) -> Void = { _, _ in }
        var onTap: (CampusMapPoint, String?) -> Void = { _, _ in }
        var onEntrance: (String) -> Void = { _ in }
        var animatesCamera = true
        var insets = UIEdgeInsets.zero
        var levelId: String?
        var appliedFraming: UInt64 = 0
        var pendingFraming: CampusMapFraming?
        private var framed = false
        private var userMoving = false
        private var appearance: String?
        private var overlays: [String: MKOverlay] = [:]
        private var marks: [String: FloorMark] = [:]

        func applyFraming(to map: MKMapView) {
            guard let framing = pendingFraming, map.bounds.width > 0, map.bounds.height > 0 else { return }
            pendingFraming = nil
            appliedFraming = framing.key
            let bounds = framing.bounds
            let corner = MKMapPoint(CLLocationCoordinate2D(latitude: bounds.north, longitude: bounds.west))
            let opposite = MKMapPoint(CLLocationCoordinate2D(latitude: bounds.south, longitude: bounds.east))
            let rect = MKMapRect(
                x: min(corner.x, opposite.x), y: min(corner.y, opposite.y),
                width: abs(opposite.x - corner.x), height: abs(opposite.y - corner.y))
            map.setVisibleMapRect(rect, edgePadding: insets, animated: framed && animatesCamera)
            framed = true
        }

        // MARK: Camera and taps

        private static func gestureActive(_ map: MKMapView) -> Bool {
            let recognizers = (map.subviews.first?.gestureRecognizers ?? []) + (map.gestureRecognizers ?? [])
            return recognizers.contains { [.began, .changed, .ended].contains($0.state) }
        }

        func mapView(_ map: MKMapView, regionWillChangeAnimated animated: Bool) {
            userMoving = Self.gestureActive(map)
        }

        func mapView(_ map: MKMapView, regionDidChangeAnimated animated: Bool) {
            let region = map.region
            let camera = CampusMapCamera(
                center: CampusMapPoint(latitude: region.center.latitude, longitude: region.center.longitude),
                bounds: CampusMapBounds(
                    south: region.center.latitude - region.span.latitudeDelta / 2,
                    west: region.center.longitude - region.span.longitudeDelta / 2,
                    north: region.center.latitude + region.span.latitudeDelta / 2,
                    east: region.center.longitude + region.span.longitudeDelta / 2),
                widthPoints: Double(map.bounds.width),
                byUser: userMoving)
            userMoving = false
            DispatchQueue.main.async { [onCamera, appliedFraming] in
                onCamera(camera, appliedFraming)
            }
        }

        func gestureRecognizer(
            _ gestureRecognizer: UIGestureRecognizer,
            shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer
        ) -> Bool { true }

        @objc func tapped(_ recognizer: UITapGestureRecognizer) {
            guard recognizer.state == .ended, let map = recognizer.view as? MKMapView else { return }
            let location = recognizer.location(in: map)
            var hit = map.hitTest(location, with: nil)
            while let view = hit {
                if let annotation = view as? MKAnnotationView {
                    guard let mark = annotation.annotation as? FloorMark else { return }
                    if case .entrance = mark.record.kind { return }
                }
                hit = view.superview
            }
            let coordinate = map.convert(location, toCoordinateFrom: map)
            onTap(
                CampusMapPoint(latitude: coordinate.latitude, longitude: coordinate.longitude),
                levelId)
        }

        func mapView(_ map: MKMapView, didSelect view: MKAnnotationView) {
            if let mark = view.annotation as? FloorMark, case let .entrance(id, _, _) = mark.record.kind {
                onEntrance(id)
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

/// `setVisibleMapRect` needs a real size, which a new map does not have yet.
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
/// entrances remain visible.
private final class FloorMarkView: MKAnnotationView {
    static let reuse = "floor-mark"
    private static let iconSide: CGFloat = 28
    private let label = UILabel()
    private let icon = UIImageView()

    override init(annotation: MKAnnotation?, reuseIdentifier: String?) {
        super.init(annotation: annotation, reuseIdentifier: reuseIdentifier)
        collisionMode = .rectangle
        canShowCallout = false
        isAccessibilityElement = true
        addSubview(label)
        addSubview(icon)
        icon.contentMode = .scaleAspectFit
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { nil }

    func show(_ mark: FloorMark, traits: UITraitCollection) {
        annotation = mark
        let color = { (role: Color) in UIColor(role).resolvedColor(with: traits) }
        switch mark.record.kind {
        case let .room(name, highlighted):
            icon.isHidden = true
            label.isHidden = false
            backgroundColor = .clear
            layer.borderWidth = 0
            label.text = name
            label.font = .systemFont(ofSize: highlighted ? 13 : 10, weight: highlighted ? .bold : .medium)
            label.textColor = color(highlighted ? Theme.accent : Theme.muted)
            label.sizeToFit()
            frame.size = label.bounds.size
            label.frame.origin = .zero
            displayPriority = highlighted ? .required : .defaultLow
            zPriority = highlighted ? .max : .defaultUnselected
            accessibilityLabel = name
            accessibilityTraits = .staticText
        case let .amenity(key):
            showIcon(key, tint: color(Theme.ink), traits: traits)
            displayPriority = .defaultHigh
            zPriority = .defaultUnselected
            accessibilityLabel = t("campusMap.amenity.\(key)")
            accessibilityTraits = .image
        case let .entrance(_, name, selected):
            showIcon("entrance", tint: color(selected ? Theme.accentInk : Theme.accent), traits: traits)
            if selected { backgroundColor = color(Theme.accent) }
            displayPriority = .required
            zPriority = selected ? .max : .defaultSelected
            accessibilityLabel = name ?? t("campusMap.entrance")
            accessibilityTraits = .button
        }
    }

    private func showIcon(_ name: String, tint: UIColor, traits: UITraitCollection) {
        label.isHidden = true
        icon.isHidden = false
        icon.image = UIImage(named: "CampusMap/\(name)")?.withRenderingMode(.alwaysTemplate)
        icon.tintColor = tint
        frame.size = CGSize(width: Self.iconSide, height: Self.iconSide)
        backgroundColor = UIColor(Theme.surface).resolvedColor(with: traits)
        layer.cornerRadius = Self.iconSide / 2
        layer.borderWidth = 1
        layer.borderColor = UIColor(Theme.line).resolvedColor(with: traits).cgColor
        icon.frame = bounds.insetBy(dx: 5, dy: 5)
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

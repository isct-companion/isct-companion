import IsctCore
import MapKit
import SwiftUI

/// A box the map should bring into view; a new `key` moves the camera.
struct CampusMapFraming: Equatable {
    let key: Int
    let south: Double
    let west: Double
    let north: Double
    let east: Double

    init(key: Int, bounds: CampusMapBounds) {
        self.key = key
        south = bounds.south
        west = bounds.west
        north = bounds.north
        east = bounds.east
    }
}

/// Where the camera settled, for Core's focus rule.
struct CampusMapCameraReport {
    let center: CampusMapPoint
    let bounds: CampusMapBounds
    let widthPoints: Double
    /// A gesture moved it; the app's own moves never re-pick the building.
    let byUser: Bool
}

/// Apple's base map with every building's footprint and, for the building in
/// focus, one floor drawn over it. `MKMapView` rather than SwiftUI's `Map`: a
/// floor can have thousands of lines, which one multi-polyline overlay draws
/// with a single renderer, and line widths are metres here, which needs a
/// renderer of our own. The map owns every gesture; the page is presented
/// full screen, so no edge swipe competes with panning.
struct FloorMapView: UIViewRepresentable {
    let buildings: [CampusMapBuilding]
    /// The footprint drawn emphasised: the building in focus or the target's.
    let emphasizedBuildingId: String?
    let floor: CampusMapFloor?
    let highlighted: Set<String>
    /// The ways into the building in focus.
    let entrances: [CampusMapEntrance]
    let selectedEntranceId: String?
    let framing: CampusMapFraming?
    /// Room the room card takes at the bottom, kept clear when framing.
    let bottomInset: CGFloat
    let showsPosition: Bool
    let animatesCamera: Bool
    let onCamera: (CampusMapCameraReport) -> Void
    let onTap: (CampusMapPoint) -> Void
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
        context.coordinator.tap = tap
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
        coordinator.animatesCamera = animatesCamera
        coordinator.bottomInset = bottomInset
        if map.showsUserLocation != showsPosition { map.showsUserLocation = showsPosition }
        let appearance = "\(context.environment.colorScheme)|\(context.environment.isctTheme)"
        let footprints = ([emphasizedBuildingId ?? "", appearance] + buildings.map(\.id)).joined(separator: "|")
        if footprints != coordinator.footprintScene {
            coordinator.footprintScene = footprints
            coordinator.drawFootprints(buildings, emphasized: emphasizedBuildingId, on: map)
        }
        let floorScene = [floor?.levelId ?? "", highlighted.sorted().joined(separator: ","), appearance]
            .joined(separator: "|")
        if floorScene != coordinator.floorScene {
            coordinator.floorScene = floorScene
            coordinator.draw(floor, highlighted: highlighted, on: map)
        }
        let entranceScene = (entrances.map(\.id) + [selectedEntranceId ?? "", appearance]).joined(separator: "|")
        if entranceScene != coordinator.entranceScene {
            coordinator.entranceScene = entranceScene
            coordinator.drawEntrances(entrances, selected: selectedEntranceId, on: map)
        }
        if let framing, framing != coordinator.framing {
            coordinator.framing = framing
            coordinator.pendingFraming = framing
            coordinator.applyFraming(to: map)
        }
    }

    final class Coordinator: NSObject, MKMapViewDelegate, UIGestureRecognizerDelegate {
        var onCamera: (CampusMapCameraReport) -> Void = { _ in }
        var onTap: (CampusMapPoint) -> Void = { _ in }
        var onEntrance: (String) -> Void = { _ in }
        var animatesCamera = true
        var bottomInset: CGFloat = 0
        var footprintScene: String?
        var floorScene: String?
        var entranceScene: String?
        var framing: CampusMapFraming?
        var pendingFraming: CampusMapFraming?
        weak var tap: UITapGestureRecognizer?
        /// Whether the change under way started from a finger on the map.
        private var userMoving = false
        /// The first framing is not animated: the page opens on the room.
        private var framed = false
        private var footprintOverlays: [MKOverlay] = []
        private var floorOverlays: [MKOverlay] = []
        private var floorMarks: [MKAnnotation] = []
        private var entranceMarks: [MKAnnotation] = []
        /// However far in, a room keeps some of its surroundings in view.
        private static let leastSideMeters = 60.0

        func applyFraming(to map: MKMapView) {
            guard let framing = pendingFraming, map.bounds.width > 0, map.bounds.height > 0 else { return }
            pendingFraming = nil
            let corner = MKMapPoint(CLLocationCoordinate2D(latitude: framing.north, longitude: framing.west))
            let opposite = MKMapPoint(CLLocationCoordinate2D(latitude: framing.south, longitude: framing.east))
            let least = Self.leastSideMeters * MKMapPointsPerMeterAtLatitude(framing.north)
            let width = max(abs(opposite.x - corner.x), least)
            let height = max(abs(opposite.y - corner.y), least)
            let rect = MKMapRect(
                x: (corner.x + opposite.x - width) / 2,
                y: (corner.y + opposite.y - height) / 2,
                width: width,
                height: height)
            // The level picker sits at the right, the card at the bottom.
            let padding = UIEdgeInsets(top: 72, left: 48, bottom: bottomInset + 24, right: 72)
            map.setVisibleMapRect(rect, edgePadding: padding, animated: framed && animatesCamera)
            framed = true
        }

        // MARK: Camera

        /// A pan, pinch, rotation or double tap on the map's own view.
        private static func gestureActive(_ map: MKMapView) -> Bool {
            let recognizers = (map.subviews.first?.gestureRecognizers ?? []) + (map.gestureRecognizers ?? [])
            return recognizers.contains { [.began, .changed, .ended].contains($0.state) }
        }

        func mapView(_ map: MKMapView, regionWillChangeAnimated animated: Bool) {
            userMoving = Self.gestureActive(map)
        }

        func mapView(_ map: MKMapView, regionDidChangeAnimated animated: Bool) {
            let region = map.region
            let report = CampusMapCameraReport(
                center: CampusMapPoint(latitude: region.center.latitude, longitude: region.center.longitude),
                bounds: CampusMapBounds(
                    south: region.center.latitude - region.span.latitudeDelta / 2,
                    west: region.center.longitude - region.span.longitudeDelta / 2,
                    north: region.center.latitude + region.span.latitudeDelta / 2,
                    east: region.center.longitude + region.span.longitudeDelta / 2),
                widthPoints: Double(map.bounds.width),
                byUser: userMoving)
            userMoving = false
            // Not during a view update: this reaches SwiftUI state.
            DispatchQueue.main.async { [onCamera] in onCamera(report) }
        }

        // MARK: Taps

        func gestureRecognizer(
            _ gestureRecognizer: UIGestureRecognizer,
            shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer
        ) -> Bool { true }

        /// A tap on a room selects it; a tap on a mark is the mark's.
        @objc func tapped(_ recognizer: UITapGestureRecognizer) {
            guard recognizer.state == .ended, let map = recognizer.view as? MKMapView else { return }
            let location = recognizer.location(in: map)
            var hit = map.hitTest(location, with: nil)
            while let view = hit {
                if view is MKAnnotationView { return }
                hit = view.superview
            }
            let coordinate = map.convert(location, toCoordinateFrom: map)
            onTap(CampusMapPoint(latitude: coordinate.latitude, longitude: coordinate.longitude))
        }

        func mapView(_ map: MKMapView, didSelect view: MKAnnotationView) {
            if let mark = view.annotation as? FloorMark, case let .entrance(id, _, _) = mark.kind {
                onEntrance(id)
            }
            map.deselectAnnotation(view.annotation, animated: false)
        }

        // MARK: Drawing

        /// Every building's footprint, under the floor in focus.
        func drawFootprints(_ buildings: [CampusMapBuilding], emphasized: String?, on map: MKMapView) {
            map.removeOverlays(footprintOverlays)
            let polygons = { (building: CampusMapBuilding) in building.footprint.map(Self.polygon) }
            footprintOverlays = [
                PolygonGroup(buildings.filter { $0.id != emphasized }.flatMap(polygons), style: .footprint),
                PolygonGroup(buildings.filter { $0.id == emphasized }.flatMap(polygons), style: .footprintFocus),
            ]
            map.addOverlays(footprintOverlays, level: .aboveRoads)
        }

        /// Replaces the floor: its outline, rooms and lines, then the
        /// highlighted room over them, then labels and amenity icons.
        func draw(_ floor: CampusMapFloor?, highlighted: Set<String>, on map: MKMapView) {
            map.removeOverlays(floorOverlays)
            map.removeAnnotations(floorMarks)
            floorOverlays = []
            floorMarks = []
            guard let floor else { return }
            let chosen = floor.units.filter { highlighted.contains($0.id) }
            let others = floor.units.filter { !highlighted.contains($0.id) }
            floorOverlays.append(PolygonGroup(floor.outline.map(Self.polygon), style: .outline))
            floorOverlays.append(PolygonGroup(
                others.filter { $0.category == .classroom }.map { Self.polygon($0.polygon) }, style: .classroom))
            floorOverlays.append(PolygonGroup(
                others.filter { $0.category != .classroom }.map { Self.polygon($0.polygon) }, style: .room))
            // Fittings under the walls.
            for lines in floor.lines.sorted(by: { $0.tone == .detail && $1.tone == .wall }) {
                let polylines = lines.lines.map { line -> MKPolyline in
                    let points = line.map(\.coordinate)
                    return MKPolyline(coordinates: points, count: points.count)
                }
                floorOverlays.append(LineGroup(polylines, tone: lines.tone, widthMeters: lines.widthM))
            }
            floorOverlays.append(PolygonGroup(chosen.map { Self.polygon($0.polygon) }, style: .highlight))
            map.addOverlays(floorOverlays, level: .aboveLabels)

            floorMarks = floor.units.compactMap { unit in
                unit.label.map { FloorMark(unit.center.coordinate, .room($0, highlighted: highlighted.contains(unit.id))) }
            }
            floorMarks += floor.amenities.map { FloorMark($0.point.coordinate, .amenity($0.category)) }
            map.addAnnotations(floorMarks)
        }

        func drawEntrances(_ entrances: [CampusMapEntrance], selected: String?, on map: MKMapView) {
            map.removeAnnotations(entranceMarks)
            entranceMarks = entrances.map {
                FloorMark($0.point.coordinate, .entrance($0.id, name: $0.name, selected: $0.id == selected))
            }
            map.addAnnotations(entranceMarks)
        }

        private static func polygon(_ polygon: CampusMapPolygon) -> MKPolygon {
            let rings = polygon.rings.map { $0.map(\.coordinate) }
            let holes = rings.dropFirst().map { MKPolygon(coordinates: $0, count: $0.count) }
            let outer = rings.first ?? []
            return MKPolygon(coordinates: outer, count: outer.count, interiorPolygons: Array(holes))
        }

        func mapView(_ map: MKMapView, rendererFor overlay: MKOverlay) -> MKOverlayRenderer {
            let color = { (role: Color) in UIColor(role).resolvedColor(with: map.traitCollection) }
            switch overlay {
            case let group as LineGroup:
                return MetricLineRenderer(group: group, color: color(group.tone == .wall ? Theme.ink : Theme.muted))
            case let group as PolygonGroup:
                let renderer = MKMultiPolygonRenderer(multiPolygon: group)
                switch group.style {
                case .footprint:
                    renderer.fillColor = color(Theme.surface2).withAlphaComponent(0.8)
                    renderer.strokeColor = color(Theme.line)
                    renderer.lineWidth = 1
                case .footprintFocus:
                    renderer.fillColor = color(Theme.surface)
                    renderer.strokeColor = color(Theme.accent)
                    renderer.lineWidth = 1.5
                case .outline:
                    renderer.fillColor = color(Theme.surface)
                    renderer.strokeColor = color(Theme.line)
                    renderer.lineWidth = 1
                case .room:
                    renderer.fillColor = .clear
                case .classroom:
                    renderer.fillColor = color(Theme.surface2)
                case .highlight:
                    renderer.fillColor = color(Theme.accent).withAlphaComponent(0.35)
                    renderer.strokeColor = color(Theme.accent)
                    renderer.lineWidth = 2
                }
                return renderer
            default:
                return MKOverlayRenderer(overlay: overlay)
            }
        }

        func mapView(_ map: MKMapView, viewFor annotation: MKAnnotation) -> MKAnnotationView? {
            guard let mark = annotation as? FloorMark else { return nil }
            let view = map.dequeueReusableAnnotationView(withIdentifier: FloorMarkView.reuse, for: mark)
            (view as? FloorMarkView)?.show(mark, traits: map.traitCollection)
            return view
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
    enum Style { case footprint, footprintFocus, outline, room, classroom, highlight }
    private(set) var style = Style.room

    convenience init(_ polygons: [MKPolygon], style: Style) {
        self.init(polygons)
        self.style = style
    }
}

private final class LineGroup: MKMultiPolyline {
    private(set) var tone = CampusMapLineTone.wall
    private(set) var widthMeters = 0.0

    convenience init(_ polylines: [MKPolyline], tone: CampusMapLineTone, widthMeters: Double) {
        self.init(polylines)
        self.tone = tone
        self.widthMeters = widthMeters
    }
}

/// Lines as wide as they are in the building, so they grow as the map zooms
/// in; `MKMultiPolylineRenderer` keeps one width on screen. They never get
/// thinner than a hairline, and 0 m lines stay one.
private final class MetricLineRenderer: MKOverlayRenderer {
    private let path = CGMutablePath()
    private let color: CGColor
    private let width: CGFloat
    private static let hairlinePoints: CGFloat = 0.5

    init(group: LineGroup, color: UIColor) {
        self.color = color.cgColor
        width = group.widthMeters * MKMapPointsPerMeterAtLatitude(group.coordinate.latitude)
        super.init(overlay: group)
        for line in group.polylines where line.pointCount > 1 {
            let points = line.points()
            path.move(to: point(for: points[0]))
            for index in 1..<line.pointCount {
                path.addLine(to: point(for: points[index]))
            }
        }
    }

    override func draw(_ mapRect: MKMapRect, zoomScale: MKZoomScale, in context: CGContext) {
        context.addPath(path)
        context.setStrokeColor(color)
        context.setLineCap(.round)
        context.setLineJoin(.round)
        context.setLineWidth(max(width, Self.hairlinePoints / zoomScale))
        context.strokePath()
    }
}

private final class FloorMark: NSObject, MKAnnotation {
    enum Kind {
        case room(String, highlighted: Bool)
        case amenity(CampusMapAmenityCategory)
        case entrance(String, name: String?, selected: Bool)
    }
    let coordinate: CLLocationCoordinate2D
    let kind: Kind

    init(_ coordinate: CLLocationCoordinate2D, _ kind: Kind) {
        self.coordinate = coordinate
        self.kind = kind
    }
}

/// A room's name, an amenity's icon or an entrance. MapKit drops whichever
/// collide as the map zooms out, the target room's name last of all.
private final class FloorMarkView: MKAnnotationView {
    static let reuse = "floor-mark"
    /// Icons on the map, in points (the asset catalog's SVGs).
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
        switch mark.kind {
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
        case let .amenity(category):
            showIcon(Self.asset(category), tint: color(Theme.ink), traits: traits)
            displayPriority = .defaultHigh
            zPriority = .defaultUnselected
            accessibilityLabel = Self.amenityName(category)
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

    /// A 28-point round badge with the glyph inside it.
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

    /// The asset catalog's icon for each of the export's amenity kinds.
    private static func asset(_ category: CampusMapAmenityCategory) -> String {
        switch category {
        case .elevator: "elevator"
        case .stairs: "stairs"
        case .restroom: "restroom"
        case .restroomMale: "restroom-male"
        case .restroomFemale: "restroom-female"
        case .restroomWheelchair: "restroom-accessible"
        case .aed: "aed"
        case .lounge: "lounge"
        case .loungeWomen: "lounge-women"
        case .loungeInternational: "lounge-international"
        case .restRoom: "rest-room"
        case .calmDown: "calm-down"
        case .studyRoom: "study-room"
        }
    }

    /// What VoiceOver says for each amenity.
    static func amenityName(_ category: CampusMapAmenityCategory) -> String {
        switch category {
        case .elevator: t("campusMap.amenity.elevator")
        case .stairs: t("campusMap.amenity.stairs")
        case .restroom: t("campusMap.amenity.restroom")
        case .restroomMale: t("campusMap.amenity.restroomMale")
        case .restroomFemale: t("campusMap.amenity.restroomFemale")
        case .restroomWheelchair: t("campusMap.amenity.restroomWheelchair")
        case .aed: t("campusMap.amenity.aed")
        case .lounge: t("campusMap.amenity.lounge")
        case .loungeWomen: t("campusMap.amenity.loungeWomen")
        case .loungeInternational: t("campusMap.amenity.loungeInternational")
        case .restRoom: t("campusMap.amenity.restRoom")
        case .calmDown: t("campusMap.amenity.calmDown")
        case .studyRoom: t("campusMap.amenity.studyRoom")
        }
    }
}

extension CampusMapPoint {
    var coordinate: CLLocationCoordinate2D {
        CLLocationCoordinate2D(latitude: latitude, longitude: longitude)
    }
}

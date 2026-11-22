import IsctCore
import MapKit
import SwiftUI
import UIKit
import simd

/// How the stack is seen and drawn, as on Android (`FloorMapView.kt`).
enum CampusMapStack {
    /// Degrees from straight down.
    static let pitch: CLLocationDegrees = 55
    /// The selected room's height in the stack, metres: a block tall enough
    /// to read from any angle.
    static let highlightMetres = 3.0
    /// How far the centre of the view must move, in the stack, for the
    /// person's move to count as going somewhere rather than a turn or tilt.
    static let panPoints: CGFloat = 48
    /// The ground plate at the street floor, and while a floor under it is
    /// picked.
    static let ground: CGFloat = 0.8
    static let seeThroughGround: CGFloat = 0.35
    /// The selected room on the floor picked, and on any other floor.
    static let shownRoomOpacity: CGFloat = 0.9
    static let otherFloorRoomOpacity: CGFloat = 0.5
    static let metresPerDegreeLatitude = 110_574.0
    static let metresPerDegreeLongitude = 111_320.0
}

/// Where a point at a height shows on screen, as MapKit's camera sees it.
/// MapKit projects only the ground, so a point at height `h` above ground
/// point `g` is projected through the ground point on the same line of
/// sight: seen from altitude `A` over ground point `c`, that is
/// `c + (g - c) · A / (A - h)`. Below the ground (`h` < 0) it falls short of
/// `g`, towards the camera, as it should. MapKit's camera gives its distance
/// from the centre of the view, its pitch and heading; Android works the
/// same out from MapLibre's field of view (`Sight`).
struct StackSight {
    private let map: MKMapView
    private let view: UIView
    private let target: CLLocationCoordinate2D
    private let eastMetres: Double
    private let altitude: Double
    private let cameraX: Double
    private let cameraY: Double

    init?(map: MKMapView, in view: UIView) {
        let camera = map.camera
        let distance = camera.centerCoordinateDistance
        guard distance > 0 else { return nil }
        let pitch = camera.pitch * .pi / 180
        let heading = camera.heading * .pi / 180
        self.map = map
        self.view = view
        target = camera.centerCoordinate
        altitude = distance * cos(pitch)
        // Behind the centre of the view, against the way the camera faces.
        let back = distance * sin(pitch)
        cameraX = -back * sin(heading)
        cameraY = -back * cos(heading)
        eastMetres = cos(target.latitude * .pi / 180) * CampusMapStack.metresPerDegreeLongitude
    }

    /// `nil` for a point at or above the camera, which no view shows.
    func at(_ point: CampusMapPoint, height: Double) -> CGPoint? {
        guard altitude > 0, height < altitude * 0.9 else { return nil }
        let x = (point.longitude - target.longitude) * eastMetres
        let y = (point.latitude - target.latitude) * CampusMapStack.metresPerDegreeLatitude
        let scale = altitude / (altitude - height)
        let groundX = cameraX + (x - cameraX) * scale
        let groundY = cameraY + (y - cameraY) * scale
        let ground = CLLocationCoordinate2D(
            latitude: target.latitude + groundY / CampusMapStack.metresPerDegreeLatitude,
            longitude: target.longitude + groundX / eastMetres)
        return map.convert(ground, toPointTo: view)
    }
}

/// A camera that shows a box at an angle. MapKit fits a box only from
/// straight above, so this works the angled camera out itself: a pinhole at
/// the given pitch and heading, with the focal length the map has now, is
/// moved and drawn back until the box's corners at each height all fall in
/// `room`.
enum StackFraming {
    struct Fit {
        let center: CLLocationCoordinate2D
        let distance: CLLocationDistance
    }

    static func fit(
        _ bounds: CampusMapBounds, heights: [Double], focal: Double, view: CGSize, room: CGRect,
        pitch: CLLocationDegrees, heading: CLLocationDirection
    ) -> Fit? {
        guard focal > 0, room.width > 1, room.height > 1 else { return nil }
        let latitude = (bounds.north + bounds.south) / 2
        let longitude = (bounds.east + bounds.west) / 2
        let east = cos(latitude * .pi / 180) * CampusMapStack.metresPerDegreeLongitude
        let north = CampusMapStack.metresPerDegreeLatitude
        let halfX = (bounds.east - bounds.west) / 2 * east
        let halfY = (bounds.north - bounds.south) / 2 * north
        let points = heights.flatMap { height in
            [(-halfX, -halfY), (halfX, -halfY), (halfX, halfY), (-halfX, halfY)].map {
                SIMD3<Double>($0.0, $0.1, height)
            }
        }
        let p = pitch * .pi / 180
        let t = heading * .pi / 180
        // The way the camera looks, its right and its up, in east-north-up.
        let forward = SIMD3<Double>(sin(p) * sin(t), sin(p) * cos(t), -cos(p))
        let right = SIMD3<Double>(cos(t), -sin(t), 0)
        let up = simd_cross(right, forward)
        let ahead = SIMD2<Double>(sin(t), cos(t))
        let across = SIMD2<Double>(cos(t), -sin(t))
        let middle = SIMD2<Double>(Double(view.width) / 2, Double(view.height) / 2)
        let goal = SIMD2<Double>(Double(room.midX), Double(room.midY))

        func project(_ centre: SIMD2<Double>, _ distance: Double) -> (low: SIMD2<Double>, high: SIMD2<Double>)? {
            let eye = SIMD3<Double>(centre.x, centre.y, 0) - distance * forward
            var low = SIMD2<Double>(repeating: .greatestFiniteMagnitude)
            var high = SIMD2<Double>(repeating: -.greatestFiniteMagnitude)
            for point in points {
                let seen = point - eye
                let depth = simd_dot(seen, forward)
                guard depth > 1 else { return nil }
                let screen = middle + focal / depth * SIMD2(simd_dot(seen, right), -simd_dot(seen, up))
                low = simd_min(low, screen)
                high = simd_max(high, screen)
            }
            return (low, high)
        }

        // The ground centre that puts the box's picture in the middle of
        // `room` at this distance, and that picture.
        func placed(_ distance: Double) -> (centre: SIMD2<Double>, low: SIMD2<Double>, high: SIMD2<Double>)? {
            var centre = SIMD2<Double>(0, 0)
            for _ in 0..<6 {
                guard let seen = project(centre, distance) else { return nil }
                let off = (seen.low + seen.high) / 2 - goal
                // A step right moves the picture left; a step ahead moves it
                // down, foreshortened by the pitch.
                centre += off.x * distance / focal * across
                centre -= off.y * distance / (focal * max(cos(p), 0.1)) * ahead
            }
            guard let seen = project(centre, distance) else { return nil }
            return (centre, seen.low, seen.high)
        }

        func fits(_ distance: Double) -> Bool {
            guard let seen = placed(distance) else { return false }
            let size = seen.high - seen.low
            return size.x <= Double(room.width) && size.y <= Double(room.height)
        }

        var near = 1.0
        var far = 50_000.0
        guard fits(far) else { return nil }
        for _ in 0..<40 {
            let mid = (near + far) / 2
            if fits(mid) { far = mid } else { near = mid }
        }
        guard let seen = placed(far) else { return nil }
        return Fit(
            center: CLLocationCoordinate2D(
                latitude: latitude + seen.centre.y / north, longitude: longitude + seen.centre.x / east),
            distance: far)
    }
}

/// One floor of the stack on screen: the outer ring of each of its shapes
/// at the floor's height, in the map's points. Only the floor picked brings
/// its plan; the others are their outline.
struct StackFloorPicture {
    let name: String
    let outline: [[CGPoint]]
    let walkways: [[CGPoint]]
    let rooms: [[CGPoint]]
    let shown: Bool
    let street: Bool
    /// The building has an entrance on this floor: its name says so.
    let entered: Bool
    /// False for a floor between two with plans: only where it is.
    let plan: Bool
    /// Below the street floor, under the ground.
    let under: Bool
    /// Below the floor picked.
    let below: Bool
    /// Where its name goes: by its outline's right-most corner.
    let label: CGPoint?
}

/// The selected room on one floor of the stack: each outer ring at its
/// floor and at the room's top, the same points lifted.
struct StackRoomPicture {
    let floors: [[CGPoint]]
    let tops: [[CGPoint]]
    let shown: Bool
}

/// An entrance of the floor picked and where the floor puts it on screen.
struct StackEntrancePlace {
    let id: String
    let name: String?
    let selected: Bool
    let at: CGPoint
}

/// The stack as drawn: the floors under the street, the ground (the
/// building's footprint) at the street floor, then the street floor and
/// those above, each lowest first, so a higher floor lies over a lower one.
struct StackPicture {
    let under: [StackFloorPicture]
    let ground: [[CGPoint]]
    let over: [StackFloorPicture]
    /// A floor under the street is picked: the ground is seen through.
    let seeThrough: Bool
    /// The building's sides: an upright from its lowest floor to its
    /// highest at each end of its outline as seen.
    let sides: [(CGPoint, CGPoint)]
    let rooms: [StackRoomPicture]

    /// The scene's stack for the camera as it is now, and the entrances in
    /// `visible`. Core gives entrances only to the floor picked.
    static func make(_ scene: CampusMapScene, sight: StackSight, visible: CGRect) -> (StackPicture, [StackEntrancePlace]) {
        func rings(_ polygons: [CampusMapPolygon], _ height: Double) -> [[CGPoint]] {
            polygons.compactMap { shape in
                guard let ring = shape.rings.first, ring.count >= 3 else { return nil }
                var points: [CGPoint] = []
                points.reserveCapacity(ring.count)
                for point in ring {
                    guard let at = sight.at(point, height: height) else { return nil }
                    points.append(at)
                }
                return points
            }
        }
        let stack = scene.stack
        let shownBase = stack.first(where: \.shown)?.baseM ?? 0
        let floors = stack.map { floor in
            let outline = rings(floor.outline, floor.baseM)
            return StackFloorPicture(
                name: floor.name,
                outline: outline,
                walkways: floor.shown ? rings(floor.walkways, floor.baseM) : [],
                rooms: floor.shown ? rings(floor.rooms, floor.baseM) : [],
                shown: floor.shown,
                street: floor.street,
                entered: floor.entered,
                plan: floor.hasPlan,
                under: floor.baseM < 0,
                below: floor.baseM < shownBase,
                label: outline.joined().max { $0.x < $1.x })
        }
        // The building's sides, at the outline's ends as seen from here.
        let corners = stack.flatMap { floor in floor.outline.flatMap { $0.rings.first ?? [] } }
        let lowest = stack.first?.baseM ?? 0
        let highest = stack.last?.baseM ?? 0
        let seen = corners.compactMap { point in sight.at(point, height: 0).map { (point, $0) } }
        let ends = [seen.min { $0.1.x < $1.1.x }, seen.max { $0.1.x < $1.1.x }].compactMap { $0 }
        let sides = ends.compactMap { end -> (CGPoint, CGPoint)? in
            guard let foot = sight.at(end.0, height: lowest), let head = sight.at(end.0, height: highest) else {
                return nil
            }
            return (foot, head)
        }
        let rooms = stack.compactMap { floor -> StackRoomPicture? in
            let bottoms = rings(floor.highlight, floor.baseM)
            let tops = rings(floor.highlight, floor.baseM + CampusMapStack.highlightMetres)
            guard !bottoms.isEmpty, bottoms.count == tops.count else { return nil }
            return StackRoomPicture(floors: bottoms, tops: tops, shown: floor.shown)
        }
        let picture = StackPicture(
            under: floors.filter(\.under),
            ground: rings(scene.stackGround, 0),
            over: floors.filter { !$0.under },
            seeThrough: shownBase < 0,
            sides: sides,
            rooms: rooms)
        let entrances = stack.flatMap { floor in
            floor.entrances.compactMap { entrance -> StackEntrancePlace? in
                guard let at = sight.at(entrance.point, height: floor.baseM), visible.contains(at) else { return nil }
                return StackEntrancePlace(
                    id: entrance.entranceId, name: entrance.name, selected: entrance.selected, at: at)
            }
        }
        return (picture, entrances)
    }
}

/// The stack, drawn over the map rather than in it: MapKit draws only the
/// ground. Floors are flat outlines at their heights: the floor picked
/// solid with its corridors and rooms, those below it faint, those above it
/// bare outlines, and a floor without a plan or under the street dashed.
/// The ground is a plate at the street floor over the floors under it, so
/// they read as under the ground; while one of them is picked the plate
/// thins and that floor's plan is laid over it. Each floor is named at its
/// right, with "entrance" on every floor that has one, the street floor's in
/// bold. The selected room is a block in the accent over everything, and
/// the floor picked keeps the flat map's entrance marks, which a tap or
/// VoiceOver picks. Everywhere else a touch reaches the map.
final class StackOverlayView: UIView {
    var onSelect: (String) -> Void = { _ in }
    private var picture: StackPicture?
    private var buttons: [String: StackEntranceButton] = [:]

    override init(frame: CGRect) {
        super.init(frame: frame)
        isOpaque = false
        backgroundColor = .clear
        contentMode = .redraw
        autoresizingMask = [.flexibleWidth, .flexibleHeight]
        isHidden = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { nil }

    func show(_ picture: StackPicture?, entrances: [StackEntrancePlace]) {
        self.picture = picture
        isHidden = picture == nil
        setNeedsDisplay()
        let wanted = Set(entrances.map(\.id))
        for (id, button) in buttons where !wanted.contains(id) {
            button.removeFromSuperview()
            buttons[id] = nil
        }
        for entrance in entrances {
            let button = buttons[entrance.id] ?? makeButton(entrance.id)
            button.show(entrance, traits: traitCollection)
        }
    }

    /// The theme changed: the same picture in its colours.
    func restyle() {
        setNeedsDisplay()
        for button in buttons.values { button.restyle(traits: traitCollection) }
    }

    private func makeButton(_ id: String) -> StackEntranceButton {
        let button = StackEntranceButton()
        button.addAction(UIAction { [weak self] _ in self?.onSelect(id) }, for: .primaryActionTriggered)
        addSubview(button)
        buttons[id] = button
        return button
    }

    /// Only the entrance marks take touches.
    override func hitTest(_ point: CGPoint, with event: UIEvent?) -> UIView? {
        let hit = super.hitTest(point, with: event)
        return hit === self ? nil : hit
    }

    override func draw(_ rect: CGRect) {
        guard let picture, let context = UIGraphicsGetCurrentContext() else { return }
        let traits = traitCollection
        let color = { (role: Color) in UIColor(role).resolvedColor(with: traits) }

        func path(_ ring: [CGPoint]) -> CGPath {
            let path = CGMutablePath()
            path.addLines(between: ring)
            path.closeSubpath()
            return path
        }
        func fill(_ ring: [CGPoint], _ tone: UIColor) {
            context.addPath(path(ring))
            context.setFillColor(tone.cgColor)
            context.fillPath()
        }
        func stroke(_ ring: [CGPoint], _ tone: UIColor, width: CGFloat, dashed: Bool = false) {
            context.addPath(path(ring))
            context.setStrokeColor(tone.cgColor)
            context.setLineWidth(width)
            context.setLineDash(phase: 0, lengths: dashed ? [4, 3] : [])
            context.strokePath()
            context.setLineDash(phase: 0, lengths: [])
        }
        func drawFloor(_ floor: StackFloorPicture) {
            // Floors above the one picked are outlines alone: a fill would
            // veil the plan under them.
            let shade: CGFloat =
                if !floor.plan { 0 } else if floor.shown && floor.under { 0.85 } else if floor.shown { 0.95 }
                else if floor.below { 0.35 } else { 0 }
            let edge = color(floor.shown ? Theme.ink : Theme.muted).withAlphaComponent(floor.shown ? 1 : 0.7)
            let dashed = !floor.plan || (floor.under && !floor.shown)
            for shape in floor.outline {
                if shade > 0 { fill(shape, color(Theme.surface).withAlphaComponent(shade)) }
                stroke(shape, edge, width: floor.shown ? 1.5 : 1, dashed: dashed)
            }
            for shape in floor.walkways { fill(shape, color(Theme.accentSoft)) }
            for shape in floor.rooms {
                fill(shape, color(Theme.surface2))
                stroke(shape, color(Theme.line), width: 1)
            }
        }

        context.setLineJoin(.round)
        for (foot, head) in picture.sides {
            context.move(to: foot)
            context.addLine(to: head)
            context.setStrokeColor(color(Theme.muted).withAlphaComponent(0.5).cgColor)
            context.setLineWidth(1)
            context.strokePath()
        }
        picture.under.filter { !$0.shown }.forEach(drawFloor)
        let plate = picture.seeThrough ? CampusMapStack.seeThroughGround : CampusMapStack.ground
        for shape in picture.ground {
            fill(shape, color(Theme.bg).withAlphaComponent(plate))
            stroke(shape, color(Theme.muted), width: 1.5)
        }
        // The floor picked under the street is read through the ground: its
        // plan over the plate, lower down than the plate's own edge.
        picture.under.filter(\.shown).forEach(drawFloor)
        picture.over.forEach(drawFloor)

        let entrance = t("campusMap.entrance")
        for floor in picture.under + picture.over {
            guard let at = floor.label else { continue }
            let font = UIFont.systemFont(ofSize: 12, weight: floor.shown || floor.street ? .bold : .regular)
            let tone = floor.shown ? Theme.accent : floor.plan ? Theme.ink : Theme.muted
            let name = floor.entered ? "\(floor.name) · \(entrance)" : floor.name
            // Its baseline 4 pt under the corner, as on Android.
            (name as NSString).draw(
                at: CGPoint(x: at.x + 6, y: at.y + 4 - font.ascender),
                withAttributes: [.font: font, .foregroundColor: color(tone)])
        }

        // Each room is one layer, so its faces overlap without darkening.
        let accent = color(Theme.accent)
        let roofEdge = color(Theme.surface)
        for room in picture.rooms {
            context.saveGState()
            context.setAlpha(room.shown ? CampusMapStack.shownRoomOpacity : CampusMapStack.otherFloorRoomOpacity)
            context.beginTransparencyLayer(auxiliaryInfo: nil)
            for (base, top) in zip(room.floors, room.tops) {
                fill(base, accent)
                for index in base.indices {
                    let next = (index + 1) % base.count
                    fill([base[index], base[next], top[next], top[index]], accent)
                }
                fill(top, accent)
                stroke(top, roofEdge, width: 1.5)
            }
            context.endTransparencyLayer()
            context.restoreGState()
        }
    }
}

/// An entrance mark of the floor picked, as the flat map draws it (a 28 pt
/// disc in a 44 pt touch area): it picks the entrance, and VoiceOver reads
/// its name.
final class StackEntranceButton: UIControl {
    private static let side: CGFloat = 44
    private static let mark: CGFloat = 28
    private let badge = UIView()
    private let icon = UIImageView()
    private var selectedEntrance = false

    init() {
        super.init(frame: CGRect(x: 0, y: 0, width: Self.side, height: Self.side))
        badge.isUserInteractionEnabled = false
        addSubview(badge)
        badge.addSubview(icon)
        icon.contentMode = .scaleAspectFit
        icon.image = UIImage(named: "CampusMap/entrance")?.withRenderingMode(.alwaysTemplate)
        let inset = (Self.side - Self.mark) / 2
        badge.frame = bounds.insetBy(dx: inset, dy: inset)
        badge.layer.cornerRadius = Self.mark / 2
        badge.layer.borderWidth = 1
        icon.frame = badge.bounds.insetBy(dx: Self.mark * 0.18, dy: Self.mark * 0.18)
        isAccessibilityElement = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { nil }

    func show(_ entrance: StackEntrancePlace, traits: UITraitCollection) {
        center = entrance.at
        selectedEntrance = entrance.selected
        accessibilityLabel = entrance.name ?? t("campusMap.entrance")
        accessibilityTraits = entrance.selected ? [.button, .selected] : .button
        restyle(traits: traits)
    }

    func restyle(traits: UITraitCollection) {
        let color = { (role: Color) in UIColor(role).resolvedColor(with: traits) }
        badge.backgroundColor = color(selectedEntrance ? Theme.accent : Theme.surface)
        badge.layer.borderColor = color(Theme.line).cgColor
        icon.tintColor = color(selectedEntrance ? Theme.accentInk : Theme.accent)
    }
}

/// Calls `tick` once a frame while started, so the stack keeps up with a
/// camera MapKit animates. The display link holds this, not its owner.
final class StackTicker: NSObject {
    var tick: () -> Void = {}
    private var link: CADisplayLink?

    func start() {
        guard link == nil else { return }
        let link = CADisplayLink(target: self, selector: #selector(step))
        link.add(to: .main, forMode: .common)
        self.link = link
    }

    func stop() {
        link?.invalidate()
        link = nil
    }

    @objc private func step() { tick() }
}

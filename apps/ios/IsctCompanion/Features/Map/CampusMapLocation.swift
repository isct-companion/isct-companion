import CoreLocation
import Observation

/// While-in-use permission, asked for only when the person taps the
/// position button. MapKit draws the blue dot itself; there is no
/// background location.
@Observable
final class CampusMapLocation: NSObject, CLLocationManagerDelegate {
    private(set) var status: CLAuthorizationStatus
    /// This page asked: once allowed, the map goes to the person's building.
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

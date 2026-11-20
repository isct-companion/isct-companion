import Foundation
import IsctCore
import OSLog

extension AppModel {
    nonisolated private static let mapLog = Logger(subsystem: "app.isctcompanion", category: "campus-map")

    // MARK: Campus map

    /// Whether Core's map knows a room string, or with `nil` whether a map
    /// is stored at all; false (also before Core has the map's file) means
    /// no map button. A failure also hides the button, but is not silent.
    func campusMapHas(_ room: String?) async -> Bool {
        if room?.isEmpty == true { return false }
        guard let session else { return false }
        do {
            guard case let .flag(known) = try await session.campusMapHas(room: room).value() else { return false }
            return known
        } catch SdkError.Unavailable {
            return false
        } catch is CancellationError {
            return false
        } catch {
            Self.mapLog.error("campus map has: \(Self.describe(error), privacy: .public)")
            return false
        }
    }

    /// Opens Core's map page on a room string, or on none to browse.
    func campusMapOpen(_ room: String?) async -> CampusMapOpening {
        guard let session else { return .failed(t("error.internal")) }
        do {
            guard case let .campusMapScene(scene?) = try await session.campusMapOpen(room: room).value() else {
                return .failed(t("error.internal"))
            }
            return .ready(scene)
        } catch SdkError.Unavailable {
            return .missing
        } catch {
            return .failed(Self.describe(error))
        }
    }

    /// One intent on an open map page; `nil` when nothing drawn changed or
    /// the page is gone. A failure is logged: the page keeps its last scene.
    func campusMapSend(_ page: UInt64, _ intent: CampusMapIntent) async -> CampusMapScene? {
        guard let session else { return nil }
        do {
            guard case let .campusMapScene(scene) = try await session.campusMapSend(page: page, intent: intent).value()
            else { return nil }
            return scene
        } catch {
            Self.mapLog.error("campus map send: \(Self.describe(error), privacy: .public)")
            return nil
        }
    }

    /// What a map search's text names; Core echoes the text so the page can
    /// drop answers to text since changed.
    func campusMapSearch(_ query: String) async -> Result<CampusMapSearch, CampusMapFailure> {
        guard let session else { return .failure(CampusMapFailure(message: t("error.internal"))) }
        do {
            guard case let .campusMapSearch(search) = try await session.campusMapSearch(query: query).value() else {
                return .failure(CampusMapFailure(message: t("error.internal")))
            }
            return .success(search)
        } catch {
            return .failure(CampusMapFailure(message: Self.describe(error)))
        }
    }

    /// Sends one floor photo to the maintainers' inbox; `nil` once stored.
    func campusMapUploadPhoto(buildingId: String, jpeg: Data) async -> CampusMapFailure? {
        guard let session else { return CampusMapFailure(message: t("error.internal")) }
        do {
            _ = try await session.campusMapUploadPhoto(buildingId: buildingId, jpeg: jpeg).value()
            return nil
        } catch {
            Self.mapLog.error("campus map photo: \(Self.describe(error), privacy: .public)")
            return CampusMapFailure(message: Self.describe(error))
        }
    }

    /// Drops Core's map page, and its open if it is still loading.
    func campusMapClose() {
        session?.campusMapClose()
    }
}

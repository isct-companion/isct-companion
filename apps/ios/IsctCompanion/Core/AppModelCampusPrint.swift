import Foundation
import IsctCore

extension AppModel {
    /// Sends one job to the campus printer; Core verifies whose student ID is
    /// billed, keeps only the chosen pages and words how it ended. `nil` when
    /// the session is not open or the call failed before Core answered.
    func campusPrint(_ document: Data, job: CampusPrintJob) async -> CampusPrintReport? {
        guard let session,
              case let .campusPrint(report)? = try? await session.campusPrint(document: document, job: job).value()
        else { return nil }
        return report
    }

    /// Whether a print would go through now, as the print screen opens and
    /// when the person checks again: Core verifies the student ID and tries
    /// the printer. `nil` when the session is not open.
    func campusPrinterState() async -> CampusPrinterState? {
        guard let session,
              case let .campusPrinterState(state)? = try? await session.campusPrinterState().value()
        else { return nil }
        return state
    }
}

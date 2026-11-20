import Foundation
import IsctCore

extension AppModel {
    /// "Save and connect": Core signs in with the entered details and keeps
    /// them only when the school accepted them (`.connected`); any other
    /// outcome saved nothing.
    func connect(_ profile: SchoolAuthProfile, _ credentials: SchoolCredentials) async throws -> SchoolConnectOutcome? {
        guard let session else { return nil }
        let value = try await session.connectSchool(profile: profile, credentials: credentials).value()
        await refreshAccounts()
        guard case let .schoolConnect(outcome) = value else { return nil }
        return outcome
    }

    /// Retry: Core reads the synced credentials again and makes one sign-in
    /// attempt now. The credentials stay as they are.
    func retrySchool(_ profile: SchoolAuthProfile) async throws {
        guard let session else { return }
        if case let .schoolSession(snapshot) = try await session.retrySchool(profile: profile).value() {
            apply(snapshot)
        }
        await refreshAccounts()
    }

    func schoolOtpCode() async throws -> SchoolOtpCode? {
        guard let session, case let .currentSchoolOtp(code) = try await session.schoolOtpCode().value() else {
            return nil
        }
        return code
    }

    /// The saved school OTP setup as an `otpauth://` link, read only when the
    /// person asks to add it to an authenticator or copy it.
    func otpSetupLink() async throws -> String {
        guard let session, case let .text(value) = try await session.schoolOtpLink().value() else {
            throw CancellationError()
        }
        return value
    }

    /// Reads the 70 Matrix cells from one photo, in row order (A1…J1, A2…).
    func scanMatrix(_ image: Data) async throws -> [MatrixCell] {
        guard let session else { return [] }
        ocrProgress = 0
        guard case let .recognitions(cells) = try await session.scanMatrix(image: image).value() else { return [] }
        return cells
    }

    /// Starts a live Matrix scan; its ten-second budget starts now.
    func beginMatrixCamera() throws -> MatrixCameraSession {
        guard let session else { throw SdkError.Closed }
        return try session.beginMatrixCamera()
    }

    /// Returns once Core has stopped the account's reads and removed its
    /// sign-in, the web views' school cookies included.
    /// Throws when signing out did not finish: the account stays saved, and
    /// signing out again redoes every step.
    func deleteCredentials(_ profile: SchoolAuthProfile) async throws {
        guard let session else { return }
        do {
            _ = try await session.deleteCredentials(profile: profile).value()
        } catch {
            await refreshAccounts()
            throw error
        }
        await refreshAccounts()
    }
}

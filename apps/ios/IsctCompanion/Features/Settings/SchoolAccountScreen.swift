import IsctCore
import SwiftUI
import UIKit

/// Sign in, change or forget a school account. The password goes straight to
/// Core, which signs in with it and stores it in the Keychain only once the
/// school accepted it. While the account is stuck, the page says why and
/// offers Core's Retry.
struct AccountScreen: View {
    let profile: SchoolAuthProfile
    let name: String
    @Environment(AppModel.self) private var model
    @Environment(WebRouter.self) private var web
    @Environment(Navigator.self) private var navigator
    @Environment(\.dismiss) private var dismiss
    @State private var username = ""
    @State private var password = ""
    @State private var otpSecret = ""
    @State private var matrix = ""
    @State private var saving = false
    @State private var retrying = false
    @State private var removing = false
    @State private var confirmingRemove = false
    @State private var error: String?
    /// What the last Connect did, shown until the next one.
    @State private var outcome: SchoolConnectOutcome?

    private var summary: CredentialSummary? {
        profile == .science ? model.credentials?.science : model.credentials?.tokyoTech
    }

    private var session: SchoolProfileSession? { model.profileSession(profile) }

    /// The states that wait for the person or for Core's next try, where
    /// Retry makes one attempt now with the synced credentials.

    var body: some View {
        Form {
            Group {
                Section {
                    TextField(t("settings.schoolAccounts.id"), text: $username)
                        .textContentType(.username)
                        .keyboardType(.asciiCapable)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    SecureField(summary?.saved == true ? t("settings.main.passwordEnterOnlyToChange") : t("settings.main.password"), text: $password)
                        .textContentType(.password)
                } footer: {
                    Text(t("settings.main.thePasswordIsSavedIn"))
                }
                // Pasted keys, not passwords: a second secure field makes AutoFill
                // read the form as a sign-up and offer new credentials instead of
                // the saved login. They stay out of screenshots and the app switcher.
                if profile == .science {
                    // Its own sections: the OTP state, and the import paths
                    // while no setup is saved or read.
                    OtpImportField(
                        saved: summary?.hasOtp == true,
                        verifiedAccount: summary?.saved == true,
                        canShowCode: session?.signOutPending != true,
                        setup: $otpSecret,
                        openSetup: {
                            web.showOtpSetup(username: username)
                            navigator.tab = .web
                        }
                    )
                } else {
                    MatrixImportSection(saved: summary?.hasMatrix == true, matrix: $matrix)
                }
                if let error {
                    Section { Text(error).foregroundStyle(Theme.danger) }
                } else if let outcome {
                    connectOutcome(outcome)
                } else if !saving, let session, !session.presentation.recovery.isEmpty {
                    status(session)
                }
                if let maintenance = session?.maintenance {
                    Section { Text(SchoolSessionText.maintenance(maintenance, profile: profile)).foregroundStyle(Theme.warn) }
                }
                Section {
                    Button {
                        Task { await save() }
                    } label: {
                        if saving { ProgressView() } else { Text(t("settings.main.saveAndConnect")) }
                    }
                    .disabled(saving || session?.signOutPending == true || username.isEmpty || (password.isEmpty && summary?.saved != true))
                }
                if summary?.saved == true || session?.presentation.recovery.contains(.finishSignOut) == true {
                    Section {
                        // Core first stops the account's reads, so this can take a moment.
                        Button(role: .destructive) {
                            confirmingRemove = true
                        } label: {
                            if removing { ProgressView() } else { Text(removeLabel) }
                        }
                        .disabled(removing)
                        // Removing cannot be taken back, so it asks once more.
                        .confirmationDialog(removeLabel, isPresented: $confirmingRemove, titleVisibility: .visible) {
                            Button(removeLabel, role: .destructive) { remove() }
                            Button(t("common.cancel"), role: .cancel) {}
                        }
                    }
                }
            }
            .themedRows()
        }
        .themedList()
        .navigationTitle(name)
        .navigationBarTitleDisplayMode(.inline)
        .onAppear { if username.isEmpty { username = summary?.username ?? "" } }
        .onChange(of: summary?.username) { _, owner in
            if username.isEmpty { username = owner ?? "" }
        }
    }

    private var removeLabel: String {
        t(session?.presentation.recovery.contains(.finishSignOut) == true ? "settings.schoolAccounts.finishSignOut" : "settings.main.removeSignInDetailsFrom")
    }

    private func remove() {
        removing = true
        error = nil
        Task {
            do {
                try await model.deleteCredentials(profile)
                dismiss()
            } catch {
                // Signing out did not finish: the account
                // stays saved, and this button with it.
                self.error = AppModel.describe(error)
            }
            removing = false
        }
    }

    /// What Connect did. Only `.connected` saved the entries; after anything
    /// else they stay in the form to be fixed and sent again.
    private func connectOutcome(_ outcome: SchoolConnectOutcome) -> some View {
        Section {
            switch outcome {
            case let .connected(presentation):
                Text(SchoolSessionText.line(presentation)).foregroundStyle(Theme.ok)
            case let .refused(_, _, presentation), let .needsWeb(_, _, presentation):
                Text(presentation.reasonKey.map { t($0) } ?? t(presentation.stateKey))
                    .foregroundStyle(outcomeIsRefusal(outcome) ? Theme.danger : Theme.warn)
                Text(t("schoolSession.connect.notSaved")).font(.footnote).foregroundStyle(Theme.muted)
                recoveryButtons(presentation)
            case let .unreachable(presentation), let .stopped(presentation):
                Text(SchoolSessionText.line(presentation)).foregroundStyle(Theme.warn)
            case let .notSaved(presentation):
                Text(SchoolSessionText.line(presentation)).foregroundStyle(Theme.danger)
            }
        }
    }

    private func outcomeIsRefusal(_ outcome: SchoolConnectOutcome) -> Bool {
        if case .refused = outcome { return true }
        return false
    }

    private func status(_ session: SchoolProfileSession) -> some View {
        Section {
            Text(SchoolSessionText.line(session))
                .foregroundStyle(session.state == .refused ? Theme.danger : Theme.warn)
            recoveryButtons(session.presentation)
        } footer: {
            if session.state == .refused { Text(t("schoolSession.retry.otherDevice")) }
        }
    }

    @ViewBuilder
    private func recoveryButtons(_ presentation: SignInPresentation) -> some View {
        if presentation.recovery.contains(.setUpOtpOnPortal) {
            Button(t("schoolSession.openPortal"), action: openPortal)
        }
        if presentation.recovery.contains(.openSchoolPage) {
            Button(t("settings.schoolAccounts.showPage")) {
                web.showTab(profile == .science ? .sciencePortal : .tokyoPortal)
                navigator.tab = .web
            }
        }
        if presentation.recovery.contains(.retry) {
            Button {
                Task { await retry() }
            } label: {
                if retrying { ProgressView() } else { Text(t("common.retry")) }
            }
            .disabled(retrying)
        }
    }

    /// The portal's start page as it is, where the person signs in and sets
    /// up school OTP by hand.
    private func openPortal() {
        web.showUnsigned(.sciencePortal)
        navigator.tab = .web
    }

    private func retry() async {
        retrying = true
        defer { retrying = false }
        do {
            try await model.retrySchool(profile)
        } catch {
            self.error = AppModel.describe(error)
        }
    }

    private func save() async {
        saving = true
        defer { saving = false }
        error = nil
        outcome = nil
        do {
            outcome = try await model.connect(
                profile,
                SchoolCredentials(username: username, password: password, otpSecret: otpSecret, matrix: matrix))
            // The secrets are saved now; the fields go back to "enter only to change".
            if case .connected = outcome {
                password = ""
                otpSecret = ""
                matrix = ""
            }
        } catch {
            self.error = AppModel.describe(error)
        }
    }
}

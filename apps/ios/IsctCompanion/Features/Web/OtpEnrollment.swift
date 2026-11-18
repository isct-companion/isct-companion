import IsctCore
import SwiftUI

/// Consent belongs to the native shell; Core owns registration and reset consent.
struct OtpEnrollmentStart: View {
    let openManual: () -> Void
    let openSetup: () -> Void
    @State private var consent = false

    var body: some View {
        HStack {
            Button(t("otpEnrollment.help")) { consent = true }
                .alert(t("otpEnrollment.help"), isPresented: $consent) {
                    Button(t("otpEnrollment.continue"), action: openSetup)
                    Button(t("otpEnrollment.manual"), action: openManual)
                    Button(t("common.cancel"), role: .cancel) {}
                } message: {
                    Text(t("otpEnrollment.consent"))
                }
            Button(t("otpEnrollment.manual"), action: openManual)
        }
        .buttonStyle(.borderless)
    }
}

/// Credential-free Core progress and intents, above the visible school page.
struct OtpEnrollmentNotice: View {
    let otp: SchoolOtpEnrollment
    @Environment(WebRouter.self) private var web
    @Environment(Navigator.self) private var navigator

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(t(message, ["account": otp.owner ?? ""]))
                .font(.subheadline)
            ViewThatFits(in: .horizontal) {
                HStack { actions }
                VStack(alignment: .leading) { actions }
            }
            .buttonStyle(.bordered)
            .controlSize(.small)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal)
        .padding(.vertical, 8)
    }

    @ViewBuilder private var actions: some View {
        if otp.stage == .confirmReset {
            Button(t("otpEnrollment.reset"), role: .destructive) { web.otpIntent(otp, .reset) }
        }
        if otp.stage == .confirmReset || otp.stage == .complete {
            Button(t(otp.stage == .confirmReset ? "otpEnrollment.import" : "otpEnrollment.settings")) {
                if otp.stage == .confirmReset { web.otpIntent(otp, .stop) }
                navigator.push(SettingsRoute.account(.science))
            }
        }
        if otp.stage == .saveFailed {
            Button(t("otpEnrollment.retrySave")) { web.otpIntent(otp, .retrySave) }
        }
        if otp.stage != .saving {
            Button(t(stopLabel)) { web.otpIntent(otp, .stop) }
        }
    }

    private var stopLabel: String {
        switch otp.stage {
        case .confirmReset: "otpEnrollment.keep"
        case .signIn, .checking, .settingUp: "common.cancel"
        default: "common.done"
        }
    }

    private var message: String {
        switch otp.stage {
        case .signIn: "otpEnrollment.signIn"
        case .checking: "otpEnrollment.checking"
        case .confirmReset: "otpEnrollment.confirmReset"
        case .settingUp: "otpEnrollment.settingUp"
        case .saving: "otpEnrollment.saving"
        case .complete: "otpEnrollment.complete"
        case .accountMismatch: "otpEnrollment.accountMismatch"
        case .incomplete: "otpEnrollment.incomplete"
        case .unknown: "otpEnrollment.unknown"
        case .saveFailed: "otpEnrollment.saveFailed"
        }
    }
}

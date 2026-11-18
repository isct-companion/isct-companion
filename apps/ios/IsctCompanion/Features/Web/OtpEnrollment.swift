import IsctCore
import SwiftUI

/// Credential-free Core progress and intents, above the visible school page.
struct OtpEnrollmentNotice: View {
    let otp: SchoolOtpEnrollment
    @Environment(WebRouter.self) private var web
    @Environment(Navigator.self) private var navigator

    var body: some View {
        VStack(alignment: .leading, spacing: Theme.rowGap) {
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
        .padding(.horizontal, Theme.rowX)
        .padding(.vertical, Theme.rowY)
    }

    @ViewBuilder private var actions: some View {
        if otp.stage == .complete {
            Button(t("otpEnrollment.settings")) {
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
        case .signIn, .checking, .settingUp: "common.cancel"
        default: "common.done"
        }
    }

    private var message: String {
        switch otp.stage {
        case .signIn: "otpEnrollment.signIn"
        case .checking: "otpEnrollment.checking"
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

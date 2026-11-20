package app.isctcompanion.core

import app.isctcompanion.design.t

/** "Save and connect": Core signs in with the entered details and keeps
 * them only when the school accepted them (`Connected`); every other
 * outcome saved nothing. */
internal suspend fun AppModel.connect(profile: SchoolAuthProfile, credentials: SchoolCredentials): SchoolConnectOutcome? {
    val session = session ?: return null
    val value = session.connectSchool(profile, credentials).value()
    refreshAccounts()
    return (value as? OperationResult.SchoolConnect)?.outcome
}

/** Retry on an account Core holds (`refused`, `needs-web`) or waits on
 * (`unavailable`): Core reads the synced credentials again and signs in
 * once in front; the credentials stay as they are. */
internal suspend fun AppModel.retrySchool(profile: SchoolAuthProfile) {
    val session = session ?: return
    (session.retrySchool(profile).value() as? OperationResult.SchoolSession)?.let { applySchool(it.snapshot) }
}

internal suspend fun AppModel.schoolOtpCode(): SchoolOtpCode? {
    val session = session ?: return null
    return (session.schoolOtpCode().value() as? OperationResult.CurrentSchoolOtp)?.snapshot
}

/** The saved school OTP setup as an `otpauth://` link, read only when the
 * person asks to add it to an authenticator or copy it. */
internal suspend fun AppModel.otpSetupLink(): String {
    val session = session ?: throw IllegalStateException(t("failure.thisIsNotAvailableRight"))
    return (session.schoolOtpLink().value() as? OperationResult.Text)?.value
        ?: throw IllegalStateException(t("failure.thisIsNotAvailableRight"))
}

/** Whether this build can read a Matrix card (arm64; not the x86_64 emulator). */
internal val AppModel.canScanMatrix: Boolean
    get() = runCatching { session?.capabilities()?.ocr }.getOrNull() == true

/** Reads the 70 Matrix cells from one JPEG/PNG, in row order (A1…J1, A2…). */
internal suspend fun AppModel.scanMatrix(image: ByteArray): List<MatrixCell> {
    val session = session ?: return emptyList()
    ocrProgress = 0
    return (session.scanMatrix(image).value() as? OperationResult.Recognitions)?.cells.orEmpty()
}

/** Starts a live Matrix scan; its ten-second budget starts now. */
internal fun AppModel.beginMatrixCamera(): MatrixCameraSession =
    session?.beginMatrixCamera() ?: throw SdkException.Closed()

/** Forgets an account. Returns once Core's account barrier passed: its
 * reads stopped, and its credentials, cookies and LMS token are gone, with
 * every school web sign-in (the other account's next page writes its own
 * again). */
internal suspend fun AppModel.deleteCredentials(profile: SchoolAuthProfile) {
    val session = session ?: return
    try {
        session.deleteCredentials(profile).value()
    } finally {
        refreshAccounts()
    }
}

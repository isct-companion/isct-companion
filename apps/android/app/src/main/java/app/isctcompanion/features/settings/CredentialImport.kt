package app.isctcompanion.features.settings

import app.isctcompanion.core.beginMatrixCamera
import app.isctcompanion.core.canScanMatrix
import app.isctcompanion.core.otpSetupLink
import app.isctcompanion.shared.StillCameraResult
import app.isctcompanion.shared.rememberStillCamera
import android.content.ActivityNotFoundException
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.ImageDecoder
import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.FlowRowScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.CheckCircle
import androidx.compose.material.icons.rounded.ContentCopy
import androidx.compose.material.icons.rounded.ContentPaste
import androidx.compose.material.icons.rounded.DocumentScanner
import androidx.compose.material.icons.rounded.Image
import androidx.compose.material.icons.rounded.Key
import androidx.compose.material.icons.rounded.Keyboard
import androidx.compose.material.icons.rounded.QrCodeScanner
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.AssistChip
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.isctcompanion.LocalModel
import app.isctcompanion.core.AppModel
import app.isctcompanion.core.MatrixCameraSession
import app.isctcompanion.core.MatrixConfidence
import app.isctcompanion.core.MatrixOrder
import app.isctcompanion.core.OtpImportAccount
import app.isctcompanion.core.OtpImportException
import app.isctcompanion.core.importOtp
import app.isctcompanion.core.parseMatrixText
import app.isctcompanion.design.Theme
import app.isctcompanion.design.t
import com.google.android.gms.common.moduleinstall.ModuleInstall
import com.google.android.gms.common.moduleinstall.ModuleInstallRequest
import com.google.mlkit.vision.barcode.BarcodeScannerOptions
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.codescanner.GmsBarcodeScannerOptions
import com.google.mlkit.vision.codescanner.GmsBarcodeScanning
import com.google.mlkit.vision.common.InputImage
import java.io.ByteArrayOutputStream
import kotlin.coroutines.resume
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.selection.selectable
import androidx.compose.ui.semantics.Role
import androidx.compose.material3.RadioButton
import app.isctcompanion.design.ActionButton
import app.isctcompanion.design.ButtonLevel

// Ways to bring a school second factor onto the phone. Parsing (authenticator
// links and transfers) and recognition (Matrix OCR) are Core's; the shell only
// turns the camera or a photo into text or image bytes.

@Composable
private fun SourceChip(label: String, icon: ImageVector, enabled: Boolean = true, onClick: () -> Unit) {
    AssistChip(
        onClick = onClick, enabled = enabled,
        shape = RoundedCornerShape(Theme.radiusControl),
        // One line: a chip that does not fit moves to the next row of
        // `ChipRow` instead of breaking its label into a column.
        label = { Text(label, maxLines = 1, softWrap = false) },
        leadingIcon = { Icon(icon, null, modifier = Modifier.size(18.dp)) },
    )
}

/** Source chips, wrapping onto another row when the width runs out. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun ChipRow(content: @Composable FlowRowScope.() -> Unit) =
    FlowRow(horizontalArrangement = Arrangement.spacedBy(Theme.space4), content = content)

/** An image on the clipboard (a screenshot shared to it), as a content URI. */
private fun clipboardImage(context: Context): Uri? {
    val clip = context.getSystemService(ClipboardManager::class.java)?.primaryClip ?: return null
    if (clip.itemCount == 0 || !clip.description.hasMimeType("image/*")) return null
    return clip.getItemAt(0).uri
}

private fun clipboardText(context: Context): String? =
    context.getSystemService(ClipboardManager::class.java)?.primaryClip
        ?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(context)?.toString()

// OTP

/** The OTP setup, as the desktop's chooser: once saved, one line that says
 * codes fill themselves in; otherwise a choice between an authenticator's
 * export QR (Google's code scanner, or a screenshot on this phone) and a
 * pasted setup. Enrollment itself belongs to Core; this field offers
 * the authorized setup entry or imports an existing authenticator. */
@Composable
fun OtpImportField(
    saved: Boolean,
    verifiedAccount: Boolean,
    canShowCode: Boolean,
    setup: String,
    openSetup: () -> Unit,
    onSetup: (String) -> Unit,
) {
    val context = LocalContext.current
    val model = LocalModel.current
    val scope = rememberCoroutineScope()
    var changing by remember { mutableStateOf(false) }
    // The app sets OTP up at school ("help"), or an existing setup is
    // imported ("import") from an export QR or a pasted secret (`path`).
    var branch by remember { mutableStateOf("help") }
    var confirming by remember { mutableStateOf(false) }
    var path by remember { mutableStateOf("export") }
    var copied by remember { mutableStateOf(false) }
    var choices by remember { mutableStateOf<List<OtpImportAccount>>(emptyList()) }
    // What was read: the account's name, or "" for a bare secret.
    var label by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var editing by remember { mutableStateOf(false) }

    // A good read from a scan, a photo or a paste folds the chooser into one
    // line. A typed entry read as the field loses focus stays open: that is
    // often the tap on Save and Connect, which must not move under the finger.
    fun choose(account: OtpImportAccount, fold: Boolean = true) {
        onSetup(account.setup)
        choices = emptyList()
        label = name(account)
        // What was read shows in the paste field when Change opens it again.
        branch = "import"
        path = "paste"
        changing = !fold
    }

    // `pasted` text is a setup link or secret, not a QR code, so a payload
    // Core cannot read is worded for what the person gave.
    fun read(payload: String, pasted: Boolean = false, fold: Boolean = true) {
        error = null
        try {
            val accounts = importOtp(payload)
            if (accounts.size == 1) choose(accounts.first(), fold) else choices = accounts
        } catch (failure: OtpImportException) {
            error = when (failure) {
                is OtpImportException.MultiplePages -> t("error.otp-import-multiple")
                is OtpImportException.Unsupported -> t("error.otp-import-unsupported")
                is OtpImportException.Invalid ->
                    t(if (pasted) "settings.credentialImport.notASetupLinkOrSecret" else "error.otp-import-invalid")
            }
        }
    }

    val photo = rememberLauncherForActivityResult(ActivityResultContracts.PickVisualMedia()) { uri ->
        uri ?: return@rememberLauncherForActivityResult
        scope.launch {
            val payload = qrPayload(context, uri)
            if (payload == null) error = t("settings.credentialImport.noQRCodeFoundChoose") else read(payload)
        }
    }

    if (saved && !changing && setup.isEmpty()) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Theme.space4)) {
            Icon(Icons.Rounded.CheckCircle, null, tint = Theme.colors.ok, modifier = Modifier.size(20.dp))
            Text(t(if (verifiedAccount) "settings.credentialImport.codesAreFilledInFor" else "otpEnrollment.savedOnDevice"), color = Theme.colors.ink, modifier = Modifier.weight(1f))
            TextButton(onClick = { changing = true }) { Text(t("settings.credentialImport.change")) }
        }
        if (canShowCode) SchoolOtpCodeButton()
        // A QR on this screen cannot be scanned by this phone, so the
        // `otpauth://` link goes straight to the installed authenticator.
        if (verifiedAccount && canShowCode) {
            ChipRow {
                SourceChip(t("settings.credentialImport.addToAuthenticator"), Icons.Rounded.Key) {
                    scope.launch {
                        error = null
                        runCatching { model.otpSetupLink() }
                            .onSuccess { link ->
                                try {
                                    context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(link)))
                                } catch (missing: ActivityNotFoundException) {
                                    error = t("settings.credentialImport.noAuthenticatorAppOnThis")
                                }
                            }
                            .onFailure { error = AppModel.describe(it) }
                    }
                }
                SourceChip(if (copied) t("settings.credentialImport.copied") else t("settings.credentialImport.copySetup"), Icons.Rounded.ContentCopy) {
                    scope.launch {
                        error = null
                        runCatching { model.otpSetupLink() }
                            .onSuccess { link ->
                                copySensitiveOtp(context, t("settings.credentialImport.otpSetup"), link, 120_000)
                                copied = true
                                delay(2_000)
                                copied = false
                            }
                            .onFailure { error = AppModel.describe(it) }
                    }
                }
            }
        }
        Hint(t(if (verifiedAccount) "settings.credentialImport.ifYouResetOTPAt" else "otpEnrollment.connectBeforeExport"))
        error?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = Theme.colors.danger) }
        return
    }
    val read = label?.takeIf { setup.isNotEmpty() }
        ?.let { t("settings.credentialImport.readAccountReadyOnceConnected", "account" to it.ifEmpty { t("settings.credentialImport.otpSecret") }) }
    if (read != null && !changing) {
        // A good read folds the chooser into one line, like a saved setup;
        // Change opens it again to fix a wrong image or link.
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Theme.space4)) {
            Icon(Icons.Rounded.CheckCircle, null, tint = Theme.colors.ok, modifier = Modifier.size(20.dp))
            Text(read, color = Theme.colors.ok, modifier = Modifier.weight(1f))
            TextButton(onClick = { changing = true }) { Text(t("settings.credentialImport.change")) }
        }
        return
    }
    Column(verticalArrangement = Arrangement.spacedBy(Theme.space4)) {
        OtpBranchCard(branch == "help", t("otpEnrollment.help"), t("otpEnrollment.helpHint"), { branch = "help" }) {
            ActionButton(t("otpEnrollment.start"), { confirming = true }, level = ButtonLevel.Secondary)
        }
        OtpBranchCard(branch == "import", t("otpEnrollment.import"), t("otpEnrollment.importHint"), { branch = "import" }) {
            Choices(listOf("export" to t("settings.credentialImport.sourceAuthenticator"), "paste" to t("settings.credentialImport.sourceTitechApp")), path) { path = it }
            if (path == "export") {
                ChipRow {
                    SourceChip(t("settings.credentialImport.scanQR"), Icons.Rounded.QrCodeScanner) {
                        scope.launch {
                            when (val result = scanQr(context)) {
                                null -> Unit
                                else -> result.fold(::read) { error = t("settings.credentialImport.couldNotOpenTheScanner") }
                            }
                        }
                    }
                    SourceChip(t("settings.credentialImport.fromPhoto"), Icons.Rounded.Image) {
                        photo.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly))
                    }
                }
                SourceHint(t("settings.credentialImport.exportFromGoogleAuthenticator"), OtpGuide.Google)
            } else {
                OutlinedTextField(
                    setup, { onSetup(it); label = null; error = null }, singleLine = true,
                    label = { Text(t("settings.credentialImport.otpauthOrSecretKey")) },
                    // Masked like a password, but not typed as one: Autofill must not offer
                    // the school login here.
                    visualTransformation = PasswordVisualTransformation(),
                    keyboardOptions = KeyboardOptions(
                        capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false, keyboardType = KeyboardType.Ascii,
                        imeAction = ImeAction.Done,
                    ),
                    // A finished entry is read at once, like a paste; saving is only
                    // for whether the school accepts it.
                    keyboardActions = KeyboardActions(onDone = { if (setup.isNotBlank() && label == null && error == null) read(setup, pasted = true, fold = false) }),
                    modifier = Modifier.fillMaxWidth().onFocusChanged { focus ->
                        if (editing && !focus.isFocused && setup.isNotBlank() && label == null && error == null) read(setup, pasted = true, fold = false)
                        editing = focus.isFocused
                    },
                )
                SourceChip(t("settings.credentialImport.paste"), Icons.Rounded.ContentPaste) {
                    clipboardText(context)?.let { read(it, pasted = true) } ?: run { error = t("settings.credentialImport.theClipboardIsEmpty") }
                }
                SourceHint(t("settings.credentialImport.pasteFromTitechApp"), OtpGuide.TitechApp)
            }
        }
    }
    read?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = Theme.colors.ok) }
    error?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = Theme.colors.danger) }
    // Opened with Change: fold back to the setup just read, or drop a wrong
    // entry and keep the saved one.
    if (changing && read != null) {
        TextButton(onClick = { changing = false }) { Text(t("settings.credentialImport.collapse")) }
    }
    if (saved) {
        TextButton(onClick = {
            onSetup("")
            label = null
            error = null
            changing = false
        }) { Text(t("settings.credentialImport.keepTheSavedSetup")) }
    }
    // Asked once, before the school page opens, whatever the school has:
    // Core then replaces an existing setup without asking again.
    if (confirming) {
        AlertDialog(
            onDismissRequest = { confirming = false },
            title = { Text(t("otpEnrollment.help")) },
            text = { Text(t("otpEnrollment.confirm")) },
            confirmButton = {
                TextButton(onClick = { confirming = false; openSetup() }) {
                    Text(t("otpEnrollment.start"), color = Theme.colors.danger)
                }
            },
            dismissButton = { TextButton(onClick = { confirming = false }) { Text(t("common.cancel")) } },
        )
    }
    if (choices.isNotEmpty()) {
        AlertDialog(
            onDismissRequest = { choices = emptyList() },
            title = { Text(t("settings.credentialImport.whichAccountShouldBeUsed")) },
            text = {
                Column {
                    for (account in choices) {
                        Text(
                            name(account),
                            modifier = Modifier.fillMaxWidth().clickable { choose(account) }.padding(vertical = Theme.space6),
                        )
                    }
                }
            },
            confirmButton = {},
            dismissButton = { TextButton(onClick = { choices = emptyList() }) { Text(t("common.cancel")) } },
        )
    }
}

/** One of the two ways in, as a radio card: the title and what the way
 * does; only the chosen one shows its controls. */
@Composable
private fun OtpBranchCard(selected: Boolean, title: String, hint: String, select: () -> Unit, content: @Composable ColumnScope.() -> Unit) {
    val colors = Theme.colors
    Column(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(Theme.radiusColumn))
            .background(if (selected) colors.accentSoft else colors.surface),
    ) {
        Row(
            verticalAlignment = Alignment.Top,
            horizontalArrangement = Arrangement.spacedBy(Theme.space2),
            modifier = Modifier
                .fillMaxWidth()
                .selectable(selected, role = Role.RadioButton, onClick = select)
                .padding(end = Theme.space7, top = Theme.space1, bottom = Theme.space6),
        ) {
            RadioButton(selected, onClick = null, modifier = Modifier.padding(Theme.space6).size(20.dp))
            Column(Modifier.weight(1f).padding(top = Theme.space6), verticalArrangement = Arrangement.spacedBy(Theme.space1)) {
                Text(title, color = colors.ink, fontWeight = FontWeight.SemiBold)
                Text(hint, style = MaterialTheme.typography.bodyMedium, color = colors.muted)
            }
        }
        if (selected) {
            Column(
                verticalArrangement = Arrangement.spacedBy(Theme.space4),
                modifier = Modifier.fillMaxWidth().padding(start = 48.dp, end = Theme.space7, bottom = Theme.space7),
                content = content,
            )
        }
    }
}

/** One short line on what to bring from the source, beside the button
 * that shows the steps with pictures; the guide's last step says what not to
 * share and what to delete. */
@Composable
private fun SourceHint(text: String, guide: OtpGuide) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Theme.space4)) {
        Box(Modifier.weight(1f)) { Hint(text) }
        OtpGuideButton(guide)
    }
}

@Composable
private fun Hint(text: String) = Text(text, style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted)

private fun name(account: OtpImportAccount) =
    listOf(account.issuer, account.label).filter { it.isNotEmpty() }.joinToString(" · ")

/** Google's code scanner: its own camera UI from Play services, no camera
 * permission here. `null` when the person backs out. */
private suspend fun scanQr(context: Context): Result<String>? = suspendCancellableCoroutine { done ->
    val options = GmsBarcodeScannerOptions.Builder().setBarcodeFormats(Barcode.FORMAT_QR_CODE).build()
    val scanner = GmsBarcodeScanning.getClient(context, options)
    // The scanner module downloads on first use; ask for it up front.
    ModuleInstall.getClient(context).installModules(ModuleInstallRequest.newBuilder().addApi(scanner).build())
    scanner.startScan()
        .addOnSuccessListener { code -> done.resume(code.rawValue?.let { Result.success(it) }) }
        .addOnCanceledListener { done.resume(null) }
        .addOnFailureListener { done.resume(Result.failure(it)) }
}

/** The first QR code ML Kit finds in a picked image. */
private suspend fun qrPayload(context: Context, uri: Uri): String? = suspendCancellableCoroutine { done ->
    val image = runCatching { InputImage.fromFilePath(context, uri) }.getOrNull()
    if (image == null) {
        done.resume(null)
        return@suspendCancellableCoroutine
    }
    val scanner = BarcodeScanning.getClient(BarcodeScannerOptions.Builder().setBarcodeFormats(Barcode.FORMAT_QR_CODE).build())
    scanner.process(image)
        .addOnSuccessListener { codes -> done.resume(codes.firstNotNullOfOrNull { it.rawValue }) }
        .addOnFailureListener { done.resume(null) }
}

// Matrix

/** The Matrix card: photograph it (or pick a photo), Core reads the 70 cells,
 * and the person checks the grid against the card before saving. Pasting all
 * 70 letters (copied from the desktop app) fills the grid too. */
@Composable
fun MatrixImportSection(saved: Boolean, onMatrix: (String) -> Unit) {
    val model = LocalModel.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val cells = remember { mutableStateListOf(*Array(70) { "" }) }
    val confidence = remember { mutableStateListOf<MatrixConfidence?>(*Array(70) { null }) }
    var editing by remember { mutableStateOf(false) }
    var reading by remember { mutableStateOf(false) }
    var note by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    val canScan = model.canScanMatrix

    fun publish() = onMatrix(cells.joinToString(""))

    // Core reads letters along rows, or titech-app's export (down columns).
    fun paste(text: String) {
        runCatching { parseMatrixText(text) }
            .onSuccess { pasted ->
                error = null
                pasted.cells.forEachIndexed { index, c ->
                    cells[index] = c.toString()
                    confidence[index] = null
                }
                note = if (pasted.order == MatrixOrder.COLUMNS) t("settings.credentialImport.matrixImportedFromTitechApp") else null
                publish()
                editing = true
            }
            .onFailure { error = t("settings.credentialImport.pasteExactly70LettersIn") }
    }

    suspend fun scanImage(uri: Uri) {
        reading = true
        error = null
        note = null
        try {
            val jpeg = withContext(Dispatchers.IO) { jpeg(context, uri) }
            val result = model.scanMatrix(jpeg)
            if (result.size != 70) {
                error = t("settings.credentialImport.couldNotFindTheGrid")
            } else {
                result.forEachIndexed { index, cell ->
                    cells[index] = cell.text
                    confidence[index] = cell.level
                }
                publish()
                editing = true
                note = t("settings.credentialImport.checkTheColouredCellsAgainst")
            }
        } catch (failure: Exception) {
            error = AppModel.describe(failure)
        }
        reading = false
    }

    fun scan(uri: Uri) {
        scope.launch { scanImage(uri) }
    }

    var live by remember { mutableStateOf<MatrixCameraSession?>(null) }
    live?.let { session ->
        MatrixCameraDialog(session) { result ->
            live = null
            // The final state of a live scan fills the same draft as a photo.
            if (result != null && result.size == 70) {
                result.forEachIndexed { index, cell ->
                    cells[index] = cell.text
                    confidence[index] = cell.level
                }
                publish()
                editing = true
                note = t("settings.credentialImport.checkTheColouredCellsAgainst")
            }
        }
    }
    val photo = rememberLauncherForActivityResult(ActivityResultContracts.PickVisualMedia()) { uri ->
        uri?.let(::scan)
    }
    var choosing by remember { mutableStateOf(false) }
    val camera = rememberStillCamera<Unit> { _, result ->
        when (result) {
            is StillCameraResult.Captured -> scanImage(result.uri)
            StillCameraResult.Denied -> error = t("settings.matrixCamera.cameraAccessWasDeniedAllow")
            StillCameraResult.Cancelled -> Unit
        }
    }

    if (!editing) {
        Text(if (saved) t("settings.credentialImport.saved") else t("settings.studyProfile.notSet"), color = if (saved) Theme.colors.ok else Theme.colors.muted)
    }
    ChipRow {
        if (canScan) {
            if (camera.available) {
                SourceChip(t("settings.credentialImport.autoScan"), Icons.Rounded.DocumentScanner, enabled = !reading) {
                    error = null
                    note = null
                    runCatching { model.beginMatrixCamera() }
                        .onSuccess { live = it }
                        .onFailure { error = AppModel.describe(it) }
                }
            }
            Box {
                SourceChip(t("settings.credentialImport.uploadImage"), Icons.Rounded.Image, enabled = !reading) { choosing = true }
                DropdownMenu(expanded = choosing, onDismissRequest = { choosing = false }) {
                    if (camera.available) {
                        DropdownMenuItem(text = { Text(t("settings.credentialImport.takePhoto")) }, onClick = {
                            choosing = false
                            error = null
                            camera.take(Unit)
                        })
                    }
                    DropdownMenuItem(text = { Text(t("settings.credentialImport.chooseFromLibrary")) }, onClick = {
                        choosing = false
                        photo.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly))
                    })
                }
            }
        }
        SourceChip(t("settings.credentialImport.paste"), Icons.Rounded.ContentPaste, enabled = !reading) {
            // An image on the clipboard is read like a photo, as on iOS;
            // otherwise its text must be the 70 letters.
            clipboardImage(context)?.takeIf { canScan }?.let {
                scan(it)
                return@SourceChip
            }
            val text = clipboardText(context)
            if (text == null) {
                error = t("settings.credentialImport.theClipboardHasNoImage")
            } else {
                paste(text)
            }
        }
        if (!editing) SourceChip(t("settings.credentialImport.enterManually"), Icons.Rounded.Keyboard) { editing = true }
    }
    if (reading) {
        Text(t("settings.credentialImport.readingDone70", "done" to model.ocrProgress), style = MaterialTheme.typography.bodySmall)
        LinearProgressIndicator(progress = { model.ocrProgress / 70f }, modifier = Modifier.fillMaxWidth())
    }
    if (editing) {
        MatrixGrid(cells, confidence) { publish() }
        Row(horizontalArrangement = Arrangement.spacedBy(Theme.space6)) {
            Swatch(Theme.colors.okSoft, t("settings.credentialImport.certain"))
            Swatch(Theme.colors.warnSoft, t("settings.credentialImport.check"))
            Swatch(Theme.colors.dangerSoft, t("settings.credentialImport.unreadable"))
        }
    }
    note?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted) }
    error?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = Theme.colors.danger) }
    Text(
        if (canScan) t("settings.credentialImport.layTheCardFlatAnd")
        else t("settings.credentialImport.readingIsNotAvailableOn"),
        style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted,
    )
}

@Composable
private fun Swatch(color: Color, label: String) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Theme.space2)) {
        Box(Modifier.size(12.dp).clip(RoundedCornerShape(3.dp)).background(color))
        Text(label, style = MaterialTheme.typography.labelSmall, color = Theme.colors.muted)
    }
}

/** 7 rows × 10 columns (A–J), one letter each, tinted by Core's review level. */
@Composable
private fun MatrixGrid(cells: MutableList<String>, confidence: MutableList<MatrixConfidence?>, changed: () -> Unit) {
    val focus = remember { List(70) { FocusRequester() } }
    Column(verticalArrangement = Arrangement.spacedBy(3.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(3.dp)) {
            Box(Modifier.width(16.dp))
            for (column in 0 until 10) {
                Text(
                    ('A' + column).toString(), modifier = Modifier.weight(1f), textAlign = TextAlign.Center,
                    style = MaterialTheme.typography.labelSmall, color = Theme.colors.muted,
                )
            }
        }
        for (row in 0 until 7) {
            Row(horizontalArrangement = Arrangement.spacedBy(3.dp), verticalAlignment = Alignment.CenterVertically) {
                Text("${row + 1}", modifier = Modifier.width(16.dp), style = MaterialTheme.typography.labelSmall, color = Theme.colors.muted)
                for (column in 0 until 10) {
                    val index = row * 10 + column
                    val fill = when (confidence[index]) {
                        MatrixConfidence.HIGH -> Theme.colors.okSoft
                        MatrixConfidence.MEDIUM -> Theme.colors.warnSoft
                        MatrixConfidence.LOW -> Theme.colors.dangerSoft
                        null -> Theme.colors.surface2
                    }
                    BasicTextField(
                        value = cells[index],
                        onValueChange = { value ->
                            val letter = value.uppercase().filter { it in 'A'..'Z' }.takeLast(1)
                            cells[index] = letter
                            // A corrected cell is the person's own entry now.
                            confidence[index] = null
                            changed()
                            if (letter.isNotEmpty() && index < 69) focus[index + 1].requestFocus()
                        },
                        singleLine = true,
                        textStyle = TextStyle(
                            fontFamily = FontFamily.Monospace, fontWeight = FontWeight.SemiBold, fontSize = 15.sp,
                            textAlign = TextAlign.Center, color = Theme.colors.ink,
                        ),
                        keyboardOptions = KeyboardOptions(
                            capitalization = KeyboardCapitalization.Characters, autoCorrectEnabled = false,
                            keyboardType = KeyboardType.Ascii,
                        ),
                        modifier = Modifier
                            .weight(1f)
                            .height(32.dp)
                            .clip(RoundedCornerShape(5.dp))
                            .background(fill)
                            .padding(top = Theme.space3)
                            .focusRequester(focus[index]),
                    )
                }
            }
        }
    }
}

/** OCR reads PNG/JPEG/WebP; the decoder also applies the photo's rotation and
 * turns HEIF into JPEG. */
private fun jpeg(context: Context, uri: Uri): ByteArray {
    val bitmap = ImageDecoder.decodeBitmap(ImageDecoder.createSource(context.contentResolver, uri)) { decoder, _, _ ->
        decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
    }
    return ByteArrayOutputStream().use { out ->
        bitmap.compress(Bitmap.CompressFormat.JPEG, 95, out)
        out.toByteArray()
    }
}

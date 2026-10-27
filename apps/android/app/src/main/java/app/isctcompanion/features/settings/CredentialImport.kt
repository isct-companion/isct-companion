package app.isctcompanion.features.settings

import android.Manifest
import android.content.ActivityNotFoundException
import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.ImageDecoder
import android.net.Uri
import android.os.Handler
import android.os.Looper
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import app.isctcompanion.LocalModel
import app.isctcompanion.core.AppModel
import app.isctcompanion.core.MatrixCameraSession
import app.isctcompanion.core.MatrixConfidence
import app.isctcompanion.core.OtpImportAccount
import app.isctcompanion.core.OtpImportException
import app.isctcompanion.core.importOtp
import app.isctcompanion.core.matrixConfidence
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
import java.io.File
import kotlin.coroutines.resume
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext

// Ways to bring a school second factor onto the phone. Parsing (authenticator
// links and transfers) and recognition (Matrix OCR) are Core's; the shell only
// turns the camera or a photo into text or image bytes.

@Composable
private fun SourceChip(label: String, icon: ImageVector, enabled: Boolean = true, onClick: () -> Unit) {
    AssistChip(
        onClick = onClick, enabled = enabled, label = { Text(label) },
        leadingIcon = { Icon(icon, null, modifier = Modifier.size(18.dp)) },
    )
}

/** An image on the clipboard (a screenshot shared to it), as a content URI. */
private fun clipboardImage(context: Context): Uri? {
    val clip = context.getSystemService(ClipboardManager::class.java)?.primaryClip ?: return null
    if (clip.itemCount == 0 || !clip.description.hasMimeType("image/*")) return null
    return clip.getItemAt(0).uri
}

private fun clipboardText(context: Context): String? =
    context.getSystemService(ClipboardManager::class.java)?.primaryClip
        ?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(context)?.toString()

/** Marked sensitive so the clipboard preview and keyboard suggestions hide
 * it, and cleared after two minutes as on iOS: the link is the second factor
 * itself. A later copy of something else is left alone; when the app is out
 * of front Android hides the clipboard, and the clip is cleared regardless. */
private fun copySensitive(context: Context, text: String) {
    val clipboard = context.getSystemService(ClipboardManager::class.java) ?: return
    val clip = ClipData.newPlainText(t("OTP setup"), text)
    clip.description.extras = android.os.PersistableBundle().apply {
        putBoolean(ClipDescription.EXTRA_IS_SENSITIVE, true)
    }
    clipboard.setPrimaryClip(clip)
    // The description Android stored, stamped with the copy's time.
    val copied = clipboard.primaryClipDescription?.timestamp
    Handler(Looper.getMainLooper()).postDelayed({
        val shown = clipboard.primaryClipDescription
        if (shown == null || shown.timestamp == copied) clipboard.clearPrimaryClip()
    }, 120_000)
}

// OTP

/** The OTP setup, as the desktop's chooser: once saved, one line that says
 * codes fill themselves in; otherwise a choice between an authenticator's
 * export QR (Google's code scanner, or a screenshot on this phone) and a
 * pasted setup. Registering with the school is the desktop's job, so it is
 * pointed to. */
@Composable
fun OtpImportField(saved: Boolean, setup: String, onSetup: (String) -> Unit) {
    val context = LocalContext.current
    val model = LocalModel.current
    val scope = rememberCoroutineScope()
    var changing by remember { mutableStateOf(false) }
    var path by remember { mutableStateOf("export") }
    var copied by remember { mutableStateOf(false) }
    var choices by remember { mutableStateOf<List<OtpImportAccount>>(emptyList()) }
    var label by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }

    fun choose(account: OtpImportAccount) {
        onSetup(account.setup)
        choices = emptyList()
        label = name(account)
    }

    fun read(payload: String) {
        error = null
        val text = payload.trim()
        // A bare secret is pasted as is; Core validates it on save.
        if (!text.contains("://")) {
            onSetup(text)
            return
        }
        try {
            val accounts = importOtp(text)
            if (accounts.size == 1) choose(accounts.first()) else choices = accounts
        } catch (failure: OtpImportException) {
            error = when (failure) {
                is OtpImportException.MultiplePages -> t("Export only your school account, then load it again.")
                is OtpImportException.Unsupported -> t("This account uses a code format that is not supported.")
                is OtpImportException.Invalid -> t("This is not a QR code from a supported authenticator app.")
            }
        }
    }

    val photo = rememberLauncherForActivityResult(ActivityResultContracts.PickVisualMedia()) { uri ->
        uri ?: return@rememberLauncherForActivityResult
        scope.launch {
            val payload = qrPayload(context, uri)
            if (payload == null) error = t("No QR code found. Choose an image that shows the whole code.") else read(payload)
        }
    }

    if (saved && !changing && setup.isEmpty()) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Icon(Icons.Rounded.CheckCircle, null, tint = Theme.colors.ok, modifier = Modifier.size(20.dp))
            Text(t("Codes are filled in for you at sign-in."), color = Theme.colors.ink, modifier = Modifier.weight(1f))
            TextButton(onClick = { changing = true }) { Text(t("Change")) }
        }
        // A QR on this screen cannot be scanned by this phone, so the
        // `otpauth://` link goes straight to the installed authenticator.
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            SourceChip(t("Add to authenticator"), Icons.Rounded.Key) {
                scope.launch {
                    error = null
                    runCatching { model.otpSetupLink() }
                        .onSuccess { link ->
                            try {
                                context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(link)))
                            } catch (missing: ActivityNotFoundException) {
                                error = t("No authenticator app on this device can add this setup. Copy it instead.")
                            }
                        }
                        .onFailure { error = AppModel.describe(it) }
                }
            }
            SourceChip(if (copied) t("Copied") else t("Copy setup"), Icons.Rounded.ContentCopy) {
                scope.launch {
                    error = null
                    runCatching { model.otpSetupLink() }
                        .onSuccess { link ->
                            copySensitive(context, link)
                            copied = true
                            delay(2_000)
                            copied = false
                        }
                        .onFailure { error = AppModel.describe(it) }
                }
            }
        }
        Hint(t("If you reset OTP at school, the old school entry in your authenticator no longer works: delete it and add this one."))
        error?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = Theme.colors.danger) }
        return
    }
    Choices(listOf("export" to t("From an authenticator app"), "paste" to t("Setup link or secret")), path) { path = it }
    if (path == "export") {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            SourceChip(t("Scan QR"), Icons.Rounded.QrCodeScanner) {
                scope.launch {
                    when (val result = scanQr(context)) {
                        null -> Unit
                        else -> result.fold(::read) { error = t("Could not open the scanner. Use From photo instead.") }
                    }
                }
            }
            SourceChip(t("From photo"), Icons.Rounded.Image) {
                photo.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly))
            }
        }
        Hint(t("Google Authenticator: Transfer accounts → Export accounts, select only the school account."))
    } else {
        OutlinedTextField(
            setup, { onSetup(it); label = null }, singleLine = true,
            label = { Text(t("otpauth://… or secret key")) },
            // Masked like a password, but not typed as one: Autofill must not offer
            // the school login here.
            visualTransformation = PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(
                capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false, keyboardType = KeyboardType.Ascii,
            ),
            modifier = Modifier.fillMaxWidth(),
        )
        SourceChip(t("Paste"), Icons.Rounded.ContentPaste) {
            clipboardText(context)?.let(::read) ?: run { error = t("The clipboard is empty.") }
        }
        Hint(t("Not the six-digit code that changes every 30 seconds."))
    }
    label?.let { Text(t("Read “{{label}}”. Save to use it.", "label" to it), style = MaterialTheme.typography.bodySmall, color = Theme.colors.ok) }
    error?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = Theme.colors.danger) }
    if (!saved) {
        Hint(t("Not registered yet? The desktop app registers with the school for you."))
    } else if (setup.isEmpty()) {
        TextButton(onClick = { changing = false }) { Text(t("Keep the saved setup")) }
    }
    if (choices.isNotEmpty()) {
        AlertDialog(
            onDismissRequest = { choices = emptyList() },
            title = { Text(t("Which account should be used?")) },
            text = {
                Column {
                    for (account in choices) {
                        Text(
                            name(account),
                            modifier = Modifier.fillMaxWidth().clickable { choose(account) }.padding(vertical = 12.dp),
                        )
                    }
                }
            },
            confirmButton = {},
            dismissButton = { TextButton(onClick = { choices = emptyList() }) { Text(t("Cancel")) } },
        )
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
    val confidence = remember { mutableStateListOf<Double?>(*Array(70) { null }) }
    var editing by remember { mutableStateOf(false) }
    var reading by remember { mutableStateOf(false) }
    var note by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    val canScan = model.canScanMatrix

    fun publish() = onMatrix(cells.joinToString(""))

    fun scan(uri: Uri) {
        scope.launch {
            reading = true
            error = null
            note = null
            try {
                val jpeg = withContext(Dispatchers.IO) { jpeg(context, uri) }
                val result = model.scanMatrix(jpeg)
                if (result.size != 70) {
                    error = t("Could not find the grid. Retake the photo with the whole grid in view.")
                } else {
                    result.forEachIndexed { index, cell ->
                        cells[index] = cell.text
                        confidence[index] = cell.confidence
                    }
                    publish()
                    editing = true
                    note = t("Check the coloured cells against your card.")
                }
            } catch (failure: Exception) {
                error = AppModel.describe(failure)
            }
            reading = false
        }
    }

    var live by remember { mutableStateOf<MatrixCameraSession?>(null) }
    live?.let { session ->
        MatrixCameraDialog(session) { result ->
            live = null
            // The final state of a live scan fills the same draft as a photo.
            if (result != null && result.size == 70) {
                result.forEachIndexed { index, cell ->
                    cells[index] = cell.text
                    confidence[index] = cell.confidence
                }
                publish()
                editing = true
                note = t("Check the coloured cells against your card.")
            }
        }
    }
    val photo = rememberLauncherForActivityResult(ActivityResultContracts.PickVisualMedia()) { uri ->
        uri?.let(::scan)
    }
    // Upload image, as on iOS: the system camera's still or a library photo.
    // The system camera writes into a file this app hands it.
    var choosing by remember { mutableStateOf(false) }
    var shot by remember { mutableStateOf<Uri?>(null) }
    val hasCamera = remember { context.packageManager.hasSystemFeature(PackageManager.FEATURE_CAMERA_ANY) }
    val takePicture = rememberLauncherForActivityResult(ActivityResultContracts.TakePicture()) { taken ->
        if (taken) shot?.let(::scan)
    }
    fun takePhoto() {
        val file = File(context.cacheDir, "photos").apply { mkdirs() }.resolve("matrix.jpg")
        val uri = FileProvider.getUriForFile(context, "${context.packageName}.files", file)
        shot = uri
        takePicture.launch(uri)
    }
    // An app that declares the camera permission must hold it to start the
    // system camera.
    val cameraPermission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { allowed ->
        if (allowed) takePhoto() else error = t("Camera access was denied. Allow camera access in system settings, then try again.")
    }

    if (!editing) {
        Text(if (saved) t("Saved") else t("Not set"), color = if (saved) Theme.colors.ok else Theme.colors.muted)
    }
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        if (canScan) {
            if (hasCamera) {
                SourceChip(t("Auto scan"), Icons.Rounded.DocumentScanner, enabled = !reading) {
                    error = null
                    note = null
                    runCatching { model.beginMatrixCamera() }
                        .onSuccess { live = it }
                        .onFailure { error = AppModel.describe(it) }
                }
            }
            Box {
                SourceChip(t("Upload image"), Icons.Rounded.Image, enabled = !reading) { choosing = true }
                DropdownMenu(expanded = choosing, onDismissRequest = { choosing = false }) {
                    if (hasCamera) {
                        DropdownMenuItem(text = { Text(t("Take photo")) }, onClick = {
                            choosing = false
                            error = null
                            if (ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) {
                                takePhoto()
                            } else {
                                cameraPermission.launch(Manifest.permission.CAMERA)
                            }
                        })
                    }
                    DropdownMenuItem(text = { Text(t("Choose from library")) }, onClick = {
                        choosing = false
                        photo.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly))
                    })
                }
            }
        }
        SourceChip(t("Paste"), Icons.Rounded.ContentPaste, enabled = !reading) {
            // An image on the clipboard is read like a photo, as on iOS;
            // otherwise its text must be the 70 letters.
            clipboardImage(context)?.takeIf { canScan }?.let {
                scan(it)
                return@SourceChip
            }
            val text = clipboardText(context)
            val letters = text.orEmpty().uppercase().filterNot { it.isWhitespace() }
            if (text == null) {
                error = t("The clipboard has no image or 70 letters.")
            } else if (letters.length != 70 || !letters.all { it in 'A'..'Z' }) {
                error = t("Paste exactly 70 letters in row order (A1…J1, A2…J2, …). Spaces and line breaks are ignored.")
            } else {
                error = null
                letters.forEachIndexed { index, c ->
                    cells[index] = c.toString()
                    confidence[index] = null
                }
                publish()
                editing = true
            }
        }
        if (!editing) SourceChip(t("Enter manually"), Icons.Rounded.Keyboard) { editing = true }
    }
    if (reading) {
        Text(t("Reading {{done}}/70", "done" to model.ocrProgress), style = MaterialTheme.typography.bodySmall)
        LinearProgressIndicator(progress = { model.ocrProgress / 70f }, modifier = Modifier.fillMaxWidth())
    }
    if (editing) {
        MatrixGrid(cells, confidence) { publish() }
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Swatch(Theme.colors.okSoft, t("Certain"))
            Swatch(Theme.colors.warnSoft, t("Check"))
            Swatch(Theme.colors.dangerSoft, t("Unreadable"))
        }
    }
    note?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted) }
    error?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = Theme.colors.danger) }
    Text(
        if (canScan) t("Lay the card flat and photograph the whole grid.")
        else t("Reading is not available on this device. Paste the 70 letters copied from the desktop app, or type them."),
        style = MaterialTheme.typography.bodySmall, color = Theme.colors.muted,
    )
}

@Composable
private fun Swatch(color: Color, label: String) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        Box(Modifier.size(12.dp).clip(RoundedCornerShape(3.dp)).background(color))
        Text(label, style = MaterialTheme.typography.labelSmall, color = Theme.colors.muted)
    }
}

/** 7 rows × 10 columns (A–J), one letter each, tinted by Core's review level. */
@Composable
private fun MatrixGrid(cells: MutableList<String>, confidence: MutableList<Double?>, changed: () -> Unit) {
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
                    val fill = confidence[index]?.let {
                        when (matrixConfidence(cells[index], it)) {
                            MatrixConfidence.HIGH -> Theme.colors.okSoft
                            MatrixConfidence.MEDIUM -> Theme.colors.warnSoft
                            MatrixConfidence.LOW -> Theme.colors.dangerSoft
                        }
                    } ?: Theme.colors.surface2
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
                            .padding(top = 6.dp)
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

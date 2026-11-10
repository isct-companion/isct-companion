package app.isctcompanion

import android.os.ParcelFileDescriptor
import android.print.PrintAttributes
import android.print.PrintJobId
import android.print.PrinterCapabilitiesInfo
import android.print.PrinterId
import android.print.PrinterInfo
import android.printservice.PrintJob
import android.printservice.PrintService
import android.printservice.PrinterDiscoverySession
import android.util.Log
import app.isctcompanion.core.CampusPrintJob
import app.isctcompanion.core.CampusPrintResult
import app.isctcompanion.core.CampusPrintSides
import app.isctcompanion.core.campusPrintDocumentLimit
import app.isctcompanion.design.t
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** The campus printer in the system's print dialog. Android enables an
 * installed app's print service by itself, so there is nothing to add or
 * switch on: every app's 印刷 lists the printer. The system renders the
 * document to a PDF with the dialog's paper size, page range and
 * orientation applied; this service hands that PDF and the dialog's
 * copies, sides and color to Core, which verifies the student ID the job
 * is billed to and sends it (`Session.campusPrint`). Nothing is decided
 * here and no document is kept.
 *
 * The printer holds every job until the student releases it at the device
 * and never reports back, so a job is complete once the printer accepted
 * its last copy. One that did not get there fails with Core's reason, which
 * the system shows in its print notification with its own retry. */
class CampusPrintService : PrintService() {
    private val scope = MainScope()
    /** Jobs whose document is still being read, by the coroutine reading
     * it. Once Core has the document the send is Core's and runs to its
     * end, so only these can still be cancelled. */
    private val reading = mutableMapOf<PrintJobId, Job>()

    override fun onCreatePrinterDiscoverySession(): PrinterDiscoverySession = Discovery()

    override fun onPrintJobQueued(printJob: PrintJob) {
        val id = printJob.id ?: return
        val info = printJob.info
        val data = printJob.document.data
        if (data == null || !printJob.start()) {
            data?.close()
            printJob.fail(t("campusPrint.notice.queueFailed"))
            return
        }
        val job = CampusPrintJob(
            title = info.label,
            copies = info.copies.coerceAtLeast(1).toUInt(),
            sides = when (info.attributes.duplexMode) {
                PrintAttributes.DUPLEX_MODE_NONE -> CampusPrintSides.ONE_SIDED
                PrintAttributes.DUPLEX_MODE_LONG_EDGE -> CampusPrintSides.LONG_EDGE
                PrintAttributes.DUPLEX_MODE_SHORT_EDGE -> CampusPrintSides.SHORT_EDGE
                else -> null
            },
            color = when (info.attributes.colorMode) {
                PrintAttributes.COLOR_MODE_COLOR -> true
                PrintAttributes.COLOR_MODE_MONOCHROME -> false
                else -> null
            },
        )
        reading[id] = scope.launch {
            val result = try {
                val document = withContext(Dispatchers.IO) { read(data, LIMIT) }
                reading.remove(id)
                if (document == null) CampusPrintResult.TooLarge
                else (application as IsctApplication).model.campusPrint(document, job)
            } catch (error: CancellationException) {
                throw error
            } catch (error: Exception) {
                Log.w("isct", "campus print", error)
                null
            } finally {
                reading.remove(id)
            }
            when (result) {
                is CampusPrintResult.Printed -> printJob.complete()
                else -> printJob.fail(reason(result))
            }
        }
    }

    /** The system's cancel. A document Core already has is being sent or
     * is at the printer, where only the student can drop it; that job ends
     * by itself within the send's timeouts. */
    override fun onRequestCancelPrintJob(printJob: PrintJob) {
        val read = reading.remove(printJob.id) ?: return
        read.cancel()
        printJob.cancel()
    }

    override fun onDestroy() {
        scope.cancel()
        super.onDestroy()
    }

    /** The PDF, or `null` when it is larger than [limit] bytes: reading
     * stops one byte past the limit, so a document Core would refuse is
     * never held in memory. The system's pipe is closed whatever happens. */
    private fun read(data: ParcelFileDescriptor, limit: Int): ByteArray? =
        ParcelFileDescriptor.AutoCloseInputStream(data).use { input ->
            input.readNBytes(limit + 1).takeIf { it.size <= limit }
        }

    /** Why a job did not print, in the words the desktop's print window
     * uses; `null` is a job that never reached Core. */
    private fun reason(result: CampusPrintResult?): String = when (result) {
        null, is CampusPrintResult.Printed -> t("campusPrint.notice.queueFailed")
        is CampusPrintResult.NotConnected -> t("tools.campusPrinter.saveTheTokyoTechAccount")
        is CampusPrintResult.NotStudentId -> t("tools.campusPrinter.accountIsNotAStudentId")
        is CampusPrintResult.NeedsAttention -> t("tools.campusPrinter.accountNeedsAttention")
        is CampusPrintResult.Unverified -> t("tools.campusPrinter.accountNotVerified")
        is CampusPrintResult.NotPrintable -> t("campusPrint.window.notAPdf")
        is CampusPrintResult.TooLarge -> t("campusPrint.notice.tooLarge", "megabytes" to LIMIT / (1024 * 1024))
        is CampusPrintResult.Unreachable ->
            if (result.printed == 0u) t("tools.campusPrinter.cannotReachThePrinter")
            else t("campusPrint.window.unreachableAfter", "printed" to result.printed)
        is CampusPrintResult.NotPrinted ->
            if (result.printed == 0u) t("campusPrint.window.notPrinted")
            else t("campusPrint.window.notPrintedAfter", "printed" to result.printed)
        is CampusPrintResult.Uncertain ->
            if (result.printed == 0u) t("campusPrint.window.uncertain")
            else t("campusPrint.window.uncertainAfter", "printed" to result.printed)
    }

    /** The one printer, with what the Windows system printer offers: A4 and
     * Letter, black and white by default, two-sided on the long edge by
     * default. It is always listed as idle: whether it can be reached is
     * known only by sending, and the job's failure says so. */
    private fun printer(): PrinterInfo {
        val id = generatePrinterId(PRINTER)
        val capabilities = PrinterCapabilitiesInfo.Builder(id)
            .addMediaSize(PrintAttributes.MediaSize.ISO_A4, true)
            .addMediaSize(PrintAttributes.MediaSize.NA_LETTER, false)
            .addResolution(PrintAttributes.Resolution("600", "600 dpi", 600, 600), true)
            .setColorModes(
                PrintAttributes.COLOR_MODE_MONOCHROME or PrintAttributes.COLOR_MODE_COLOR,
                PrintAttributes.COLOR_MODE_MONOCHROME,
            )
            .setDuplexModes(
                PrintAttributes.DUPLEX_MODE_NONE or PrintAttributes.DUPLEX_MODE_LONG_EDGE or
                    PrintAttributes.DUPLEX_MODE_SHORT_EDGE,
                PrintAttributes.DUPLEX_MODE_LONG_EDGE,
            )
            .setMinMargins(PrintAttributes.Margins(MARGIN, MARGIN, MARGIN, MARGIN))
            .build()
        return PrinterInfo.Builder(id, t("tools.campusPrinter.campusPrinter"), PrinterInfo.STATUS_IDLE)
            .setDescription(t("tools.campusPrinter.printsToTheKyoceraPrinter"))
            .setCapabilities(capabilities)
            .build()
    }

    /** The system asks for printers when a print dialog opens, and for one
     * printer's capabilities when it is selected; both get the same answer. */
    private inner class Discovery : PrinterDiscoverySession() {
        override fun onStartPrinterDiscovery(priorityList: List<PrinterId>) = addPrinters(listOf(printer()))
        override fun onStopPrinterDiscovery() {}
        override fun onValidatePrinters(printerIds: List<PrinterId>) = addPrinters(listOf(printer()))
        override fun onStartPrinterStateTracking(printerId: PrinterId) = addPrinters(listOf(printer()))
        override fun onStopPrinterStateTracking(printerId: PrinterId) {}
        override fun onDestroy() {}
    }

    private companion object {
        /** The printer's local id; the system remembers the last printer by it. */
        const val PRINTER = "kyocera"
        /** The most bytes Core takes for one document (64 MiB, an `Int`). */
        val LIMIT = campusPrintDocumentLimit().toInt()
        /** 1/6 inch in thousandths of an inch, the margin the Windows
         * system printer reports (4.23 mm). */
        const val MARGIN = 167
    }
}

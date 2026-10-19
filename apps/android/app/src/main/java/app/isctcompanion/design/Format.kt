package app.isctcompanion.design

import android.text.format.DateFormat
import java.time.Instant
import java.time.LocalDate
import java.time.OffsetDateTime
import java.time.ZoneId
import java.time.ZonedDateTime
import java.time.format.DateTimeFormatter
import java.time.format.TextStyle
import java.time.temporal.ChronoUnit
import kotlin.math.ceil
import kotlin.math.max
import kotlin.math.roundToInt

/** School dates are Tokyo dates wherever the phone is. */
val Tokyo: ZoneId = ZoneId.of("Asia/Tokyo")

/** Text formatting of Core's timestamps. Urgency (`tone`) comes from Core; the
 * shell only turns times into words, in the UI language. */
object Format {
    /** `木`, `Thu`, `周四` */
    fun weekday(date: LocalDate): String =
        date.dayOfWeek.getDisplayName(TextStyle.SHORT_STANDALONE, Localizer.locale)

    /** A weekday as Core numbers them, from Sunday (0). */
    fun weekday(day: Int): String =
        java.time.DayOfWeek.of(if (day.coerceIn(0, 6) == 0) 7 else day.coerceIn(0, 6))
            .getDisplayName(TextStyle.SHORT_STANDALONE, Localizer.locale)

    /** A date written from a skeleton in the UI language (`MMMdEEE` → 10月15日(木), Thu 15 Oct). */
    fun written(date: LocalDate, skeleton: String): String {
        val pattern = DateFormat.getBestDateTimePattern(Localizer.locale, skeleton)
        return DateTimeFormatter.ofPattern(pattern, Localizer.locale).format(date)
    }

    fun date(raw: String?): Instant? =
        raw?.let { runCatching { OffsetDateTime.parse(it).toInstant() }.getOrNull() }

    fun civilDate(civil: String): LocalDate? = runCatching { LocalDate.parse(civil) }.getOrNull()

    fun civil(date: LocalDate): String = date.toString()

    /** RFC 3339 in Tokyo time, as Core expects `now`. */
    fun timestamp(instant: Instant): String =
        DateTimeFormatter.ISO_OFFSET_DATE_TIME.format(instant.atZone(Tokyo).truncatedTo(ChronoUnit.SECONDS))

    fun tokyo(instant: Instant): ZonedDateTime = instant.atZone(Tokyo)

    fun today(now: Instant = Instant.now()): LocalDate = tokyo(now).toLocalDate()

    /** `10月15日(木)`, `Thu 15 Oct` */
    fun dayTitle(civil: String): String {
        val date = civilDate(civil) ?: return civil
        return written(date, "MMMdEEE")
    }

    /** `3Q` from a term id such as `2026-3Q`. */
    fun quarter(term: String): String = term.split("-").lastOrNull() ?: term

    /** `H:MM` in Tokyo time. */
    fun time(instant: Instant): String {
        val at = tokyo(instant)
        return "%d:%02d".format(at.hour, at.minute)
    }

    /** `今日 23:59`, `Tomorrow 23:59`, `10/18(日) 23:59` */
    fun due(raw: String?, now: Instant = Instant.now()): String {
        val due = date(raw) ?: return t("No deadline")
        val day = tokyo(due).toLocalDate()
        val today = today(now)
        val time = time(due)
        return when (day) {
            today -> t("Today {{time}}", "time" to time)
            today.plusDays(1) -> t("Tomorrow {{time}}", "time" to time)
            else -> "${written(day, "MdEEE")} $time"
        }
    }

    /** Minutes until `instant`, rounded up so "1 min left" lasts until the minute. */
    fun minutes(until: Instant, now: Instant): Int =
        ceil((until.toEpochMilli() - now.toEpochMilli()) / 60_000.0).toInt()

    enum class OverdueStyle { Label, Elapsed }

    /** Countdown words for a deadline chip. */
    fun countdown(raw: String?, now: Instant, overdueStyle: OverdueStyle = OverdueStyle.Label): String {
        val due = date(raw) ?: return t("No deadline")
        val minutes = minutes(due, now)
        if (minutes < 0) {
            return when (overdueStyle) {
                OverdueStyle.Label -> t("Overdue")
                OverdueStyle.Elapsed -> t("{{count}}d overdue", "count" to max(1, -minutes / (60 * 24)))
            }
        }
        if (minutes < 60) return t("{{count}} min left", "count" to max(minutes, 1))
        if (minutes < 60 * 24) return t("{{count}}h left", "count" to minutes / 60)
        if (tokyo(due).toLocalDate() == today(now).plusDays(1)) return t("Tomorrow")
        return t("{{count}}d left", "count" to (minutes / (60.0 * 24)).roundToInt())
    }

    /** `3–4限`, `Periods 3–4` */
    fun periods(value: ByteArray?): String? {
        if (value == null || value.size != 2) return null
        val first = value[0].toUByte()
        val last = value[1].toUByte()
        return t("Periods {{range}}", "range" to if (first == last) "$first" else "$first–$last")
    }
}

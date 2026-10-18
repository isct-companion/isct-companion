package app.isctcompanion.design

import java.time.Instant
import java.time.LocalDate
import java.time.OffsetDateTime
import java.time.ZoneId
import java.time.ZonedDateTime
import java.time.format.DateTimeFormatter
import java.time.temporal.ChronoUnit
import kotlin.math.ceil
import kotlin.math.max
import kotlin.math.roundToInt

/** School dates are Tokyo dates wherever the phone is. */
val Tokyo: ZoneId = ZoneId.of("Asia/Tokyo")

/** Text formatting of Core's timestamps. Urgency (`tone`) comes from Core; the
 * shell only turns times into words. */
object Format {
    private val weekdays = listOf("月", "火", "水", "木", "金", "土", "日")

    fun weekday(date: LocalDate): String = weekdays[date.dayOfWeek.value - 1]

    fun date(raw: String?): Instant? =
        raw?.let { runCatching { OffsetDateTime.parse(it).toInstant() }.getOrNull() }

    fun civilDate(civil: String): LocalDate? = runCatching { LocalDate.parse(civil) }.getOrNull()

    fun civil(date: LocalDate): String = date.toString()

    /** RFC 3339 in Tokyo time, as Core expects `now`. */
    fun timestamp(instant: Instant): String =
        DateTimeFormatter.ISO_OFFSET_DATE_TIME.format(instant.atZone(Tokyo).truncatedTo(ChronoUnit.SECONDS))

    fun tokyo(instant: Instant): ZonedDateTime = instant.atZone(Tokyo)

    fun today(now: Instant = Instant.now()): LocalDate = tokyo(now).toLocalDate()

    /** `10月15日（木）` */
    fun dayTitle(civil: String): String {
        val date = civilDate(civil) ?: return civil
        return "${date.monthValue}月${date.dayOfMonth}日（${weekday(date)}）"
    }

    /** `3Q` from a term id such as `2026-3Q`. */
    fun quarter(term: String): String = term.split("-").lastOrNull() ?: term

    /** `H:MM` in Tokyo time. */
    fun time(instant: Instant): String {
        val at = tokyo(instant)
        return "%d:%02d".format(at.hour, at.minute)
    }

    /** `今日 23:59`, `明日 23:59`, `10/18（日）23:59` */
    fun due(raw: String?, now: Instant = Instant.now()): String {
        val due = date(raw) ?: return "期限なし"
        val day = tokyo(due).toLocalDate()
        val today = today(now)
        val time = time(due)
        return when (day) {
            today -> "今日 $time"
            today.plusDays(1) -> "明日 $time"
            else -> "${day.monthValue}/${day.dayOfMonth}（${weekday(day)}）$time"
        }
    }

    /** Minutes until `instant`, rounded up so "あと 1 分" lasts until the minute. */
    fun minutes(until: Instant, now: Instant): Int =
        ceil((until.toEpochMilli() - now.toEpochMilli()) / 60_000.0).toInt()

    enum class OverdueStyle { Label, Elapsed }

    /** Countdown words for a deadline chip. */
    fun countdown(raw: String?, now: Instant, overdueStyle: OverdueStyle = OverdueStyle.Label): String {
        val due = date(raw) ?: return "期限なし"
        val minutes = minutes(due, now)
        if (minutes < 0) {
            return when (overdueStyle) {
                OverdueStyle.Label -> "期限切れ"
                OverdueStyle.Elapsed -> "${max(1, -minutes / (60 * 24))} 日超過"
            }
        }
        if (minutes < 60) return "あと ${max(minutes, 1)} 分"
        if (minutes < 60 * 24) return "あと ${minutes / 60} 時間"
        if (tokyo(due).toLocalDate() == today(now).plusDays(1)) return "明日"
        return "あと ${(minutes / (60.0 * 24)).roundToInt()} 日"
    }

    /** `3–4限` */
    fun periods(value: ByteArray?): String? {
        if (value == null || value.size != 2) return null
        val first = value[0].toUByte()
        val last = value[1].toUByte()
        return if (first == last) "${first}限" else "${first}–${last}限"
    }
}

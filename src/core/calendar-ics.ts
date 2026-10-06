import type { TermId, Workspace } from "./model";
import { addDays, occurrencesOnDate, quarterRange } from "./schedule";

export type CalendarExportScope = "courses" | "courses-and-plans";
const escape = (value: string) =>
  value
    .replace(/\\/g, "\\\\")
    .replace(/\r?\n/g, "\\n")
    .replace(/,/g, "\\,")
    .replace(/;/g, "\\;");
const stamp = (date: string, time: string) =>
  `${date.replace(/-/g, "")}T${time.replace(/:/g, "")}00`;
/** Fold UTF-8 content lines to the iCalendar 75-octet limit. */
function fold(line: string) {
  const parts: string[] = [];
  let current = "";
  let size = 0;
  for (const char of line) {
    const bytes = new TextEncoder().encode(char).length;
    if (size + bytes > 75) {
      parts.push(current);
      current = " ";
      size = 1;
    }
    current += char;
    size += bytes;
  }
  parts.push(current);
  return parts.join("\r\n");
}
/** Export exactly the same dated occurrences shown by Today and Schedule. */
export function exportQuarterIcs(
  data: Workspace,
  term: TermId,
  scope: CalendarExportScope = "courses",
  now = new Date(),
) {
  const { from, to } = quarterRange(data, term);
  if (to < from || Date.parse(to) - Date.parse(from) > 366 * 86400000)
    throw new Error("Invalid quarter date range.");
  const lines = [
    "BEGIN:VCALENDAR",
    "VERSION:2.0",
    "PRODID:-//IsctCompanion//EN",
    "CALSCALE:GREGORIAN",
    "BEGIN:VTIMEZONE",
    "TZID:Asia/Tokyo",
    "BEGIN:STANDARD",
    "DTSTART:19700101T000000",
    "TZOFFSETFROM:+0900",
    "TZOFFSETTO:+0900",
    "TZNAME:JST",
    "END:STANDARD",
    "END:VTIMEZONE",
  ];
  for (let date = from; date <= to; date = addDays(date, 1)) {
    for (const block of occurrencesOnDate(data, term, date, {
      includePersonal: scope === "courses-and-plans",
    })) {
      if (
        block.kind !== "course" &&
        (scope !== "courses-and-plans" || block.kind !== "plan")
      )
        continue;
      lines.push(
        "BEGIN:VEVENT",
        `UID:${encodeURIComponent(block.key)}@isctcompanion.local`,
        `DTSTAMP:${now
          .toISOString()
          .replace(/[-:]/g, "")
          .replace(/\.\d{3}/, "")}`,
        `SUMMARY:${escape(block.title)}`,
        `DTSTART;TZID=Asia/Tokyo:${stamp(date, block.startTime)}`,
        `DTEND;TZID=Asia/Tokyo:${stamp(block.endDate ?? date, block.endTime)}`,
      );
      if (block.room) lines.push(`LOCATION:${escape(block.room)}`);
      lines.push("END:VEVENT");
    }
  }
  lines.push("END:VCALENDAR");
  return lines.map(fold).join("\r\n") + "\r\n";
}

import { t } from "../../i18n";
import { useState } from "react";
import { blockLanes } from "./lanes";
import {
  teachingPeriods,
  timetableOffset,
  timetablePosition,
} from "../../core/timetable-layout";

export const dayNames = [
  "Monday",
  "Tuesday",
  "Wednesday",
  "Thursday",
  "Friday",
  "Saturday",
  "Sunday",
];

const CLOCK = /^\d{1,2}:\d{2}$/;
// Classes run Monday to Saturday; Sunday has no teaching slots to pick.
const SLOT_DAYS = 6;

export type Block = {
  key: string;
  id: string;
  kind: "course" | "event" | "plan";
  title: string;
  subtitle?: string;
  room: string;
  startTime: string;
  endTime: string;
  color: string;
  conflict?: boolean;
  dim?: boolean;
};

export type DayColumn = {
  label: string;
  date?: string;
  isToday?: boolean;
  blocks: Block[];
};

/** Minutes since midnight in Japan time. */
export function jstMinutes(now = new Date()): number {
  const parts = new Intl.DateTimeFormat("en-GB", {
    timeZone: "Asia/Tokyo",
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
  }).formatToParts(now);
  const hour = Number(parts.find((p) => p.type === "hour")?.value ?? 0);
  const minute = Number(parts.find((p) => p.type === "minute")?.value ?? 0);
  return hour * 60 + minute;
}

export function WeekTimetable({
  days,
  onSelect,
  selectedId,
  showNow = false,
  initialDay = 1,
  emptyLabel = "Nothing scheduled",
  onSlot,
  activeSlots,
}: {
  days: DayColumn[];
  onSelect?: (block: Block) => void;
  selectedId?: string;
  showNow?: boolean;
  initialDay?: number;
  emptyLabel?: string;
  /** Clicking the free area of a teaching slot (day is 1 = Monday); `append` with ⌘/Ctrl. */
  onSlot?: (day: number, from: number, to: number, append: boolean) => void;
  /** Slots to highlight, as `${day}:${from}-${to}`. */
  activeSlots?: ReadonlySet<string>;
}) {
  const [day, setDay] = useState(
    Math.min(Math.max(initialDay, 1), Math.max(days.length, 1)),
  );
  const nowOffset = showNow ? timetableOffset(jstMinutes()) : undefined;
  const agenda = days[day - 1]?.blocks ?? [];
  return (
    <div className="tt" style={{ "--days": days.length } as React.CSSProperties}>
      <div className="ttGrid" role="grid" aria-label={t("Weekly timetable")}>
        <div className="ttCorner" />
        {days.map((column) => (
          <div
            className={`ttHead${column.isToday ? " today" : ""}`}
            key={column.label}
          >
            <b>{t(column.label.slice(0, 3))}</b>
            {column.date && (
              <small>{column.date.slice(5).replace("-", "/")}</small>
            )}
          </div>
        ))}
        <div className="ttAxis">
          {teachingPeriods.map((period) => {
            const position = timetablePosition(period.start, period.end);
            return (
              <div
                className={period.label === "Lunch" ? "lunch" : ""}
                key={t(period.label)}
                style={{
                  top: `${position.topPercent}%`,
                  height: `${position.heightPercent}%`,
                }}
              >
                <b>{t(period.label)}</b>
                <small>{period.start}</small>
              </div>
            );
          })}
        </div>
        {days.map((column, dayIndex) => (
          <div
            className={`ttCol${column.isToday ? " today" : ""}`}
            key={column.label}
          >
            {onSlot &&
              dayIndex < SLOT_DAYS &&
              teachingPeriods
                .filter((period) => period.periods.length > 0)
                .map((period) => {
                  const position = timetablePosition(period.start, period.end);
                  const [from, to] = period.periods as readonly [number, number];
                  const active = activeSlots?.has(`${dayIndex + 1}:${from}-${to}`);
                  return (
                    <button
                      type="button"
                      className={`ttSlot${active ? " active" : ""}`}
                      key={`slot-${period.label}`}
                      aria-pressed={Boolean(active)}
                      aria-label={`${t(column.label)} ${period.label}`}
                      title={t("Find courses at this time")}
                      style={{ top: `${position.topPercent}%`, height: `${position.heightPercent}%` }}
                      onClick={(event) => onSlot(dayIndex + 1, from, to, event.metaKey || event.ctrlKey)}
                    />
                  );
                })}
            {teachingPeriods.map((period) => {
              const position = timetablePosition(period.start, period.end);
              return (
                <i
                  className={period.label === "Lunch" ? "ttLunch" : "ttRule"}
                  key={t(period.label)}
                  style={{
                    top: `${position.topPercent}%`,
                    height:
                      period.label === "Lunch"
                        ? `${position.heightPercent}%`
                        : undefined,
                  }}
                />
              );
            })}
            {(() => {
              const lanes = blockLanes(column.blocks.filter((block) => CLOCK.test(block.startTime) && CLOCK.test(block.endTime)));
              return column.blocks.map((block) => {
              // A block without readable times is left out rather than breaking the page.
              if (!CLOCK.test(block.startTime) || !CLOCK.test(block.endTime)) return null;
              const position = timetablePosition(
                block.startTime,
                block.endTime,
              );
              const lane = lanes.get(block.key) ?? { lane: 0, lanes: 1 };
              return (
                <UiButton
                  type="button"
                  className={[
                    "ttBlock",
                    block.kind,
                    block.conflict ? "conflict" : "",
                    block.dim ? "dim" : "",
                    selectedId === block.id ? "selected" : "",
                  ]
                    .filter(Boolean)
                    .join(" ")}
                  style={
                    {
                      "--course": block.color,
                      top: `${position.topPercent}%`,
                      height: `${position.heightPercent}%`,
                      // Overlapping blocks share the column side by side.
                      "--lane": lane.lane,
                      "--lanes": lane.lanes,
                    } as React.CSSProperties
                  }
                  title={`${block.title}${block.subtitle ? ` · ${block.subtitle}` : ""} · ${block.startTime}–${block.endTime}${block.room ? ` · ${block.room}` : ""}`}
                  key={block.key}
                  onClick={() => onSelect?.(block)}
                >
                  <b>{block.title}</b>
                  {block.room && <small className="ttRoom">{block.room}</small>}
                  {block.subtitle && <span className="ttCode">{block.subtitle}</span>}
                </UiButton>
              );
              });
            })()}
            {column.isToday && nowOffset !== undefined && (
              <i className="ttNow" style={{ top: `${nowOffset}%` }} />
            )}
          </div>
        ))}
      </div>
      <div className="ttAgenda">
        <div className="ttDayTabs">
          {days.map((column, index) => (
            <UiButton
              type="button"
              key={column.label}
              className={day === index + 1 ? "active" : ""}
              onClick={() => setDay(index + 1)}
            >
              {t(column.label.slice(0, 3))}
              {column.date && <small>{column.date.slice(8)}</small>}
            </UiButton>
          ))}
        </div>
        <div className="ttAgendaList">
          {agenda.length ? (
            agenda.map((block) => (
              <UiButton
                type="button"
                className="ttAgendaRow"
                style={{ "--course": block.color } as React.CSSProperties}
                key={block.key}
                onClick={() => onSelect?.(block)}
              >
                <time>
                  {block.startTime}
                  <small>{block.endTime}</small>
                </time>
                <span>
                  <b>{block.title}</b>
                  <small>
                    {[block.subtitle, block.room].filter(Boolean).join(" · ")}
                  </small>
                </span>
              </UiButton>
            ))
          ) : (
            <div className="empty">{emptyLabel}</div>
          )}
        </div>
      </div>
    </div>
  );
}
import { UiButton } from "../mantine";

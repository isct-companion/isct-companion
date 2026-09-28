import { t } from "../../i18n";
import { useState } from "react";
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

export type Block = {
  key: string;
  id: string;
  kind: "course" | "event";
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
}: {
  days: DayColumn[];
  onSelect?: (block: Block) => void;
  selectedId?: string;
  showNow?: boolean;
  initialDay?: number;
  emptyLabel?: string;
}) {
  const [day, setDay] = useState(
    Math.min(Math.max(initialDay, 1), Math.max(days.length, 1)),
  );
  const nowOffset = showNow ? timetableOffset(jstMinutes()) : undefined;
  const agenda = days[day - 1]?.blocks ?? [];
  return (
    <div className="tt">
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
        {days.map((column) => (
          <div
            className={`ttCol${column.isToday ? " today" : ""}`}
            key={column.label}
          >
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
            {column.blocks.map((block) => {
              const position = timetablePosition(
                block.startTime,
                block.endTime,
              );
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
                    } as React.CSSProperties
                  }
                  title={`${block.title} · ${block.startTime}–${block.endTime}${block.room ? ` · ${block.room}` : ""}`}
                  key={block.key}
                  onClick={() => onSelect?.(block)}
                >
                  <b>{block.title}</b>
                  {block.subtitle && <span>{block.subtitle}</span>}
                  {block.room && <small>{block.room}</small>}
                </UiButton>
              );
            })}
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

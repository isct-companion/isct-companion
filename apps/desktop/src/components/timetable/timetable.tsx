import { t } from "../../i18n";
import { alternationLabel } from "../../lib/alternation";
import type { Alternation } from "../../lib/model";
import { useState } from "react";
import { Menu } from "../ui";
import { GuideTip } from "../guide";
import {
  axisPosition,
  axisRows,
  axisSpan,
  nowPercent,
} from "../../lib/timetable-layout";
import { jstMinutes } from "../../lib/civil-date";
import { useScheduleQuery } from "../../platform/core-query";
import type { TimetableAxis } from "../../generated/persistence";

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
  kind: "course" | "event" | "plan" | "exam";
  date?: string;
  termId?: string;
  title: string;
  subtitle?: string;
  room: string;
  startTime: string;
  endTime: string;
  color: string;
  conflict?: boolean;
  dim?: boolean;
  cancelled?: boolean;
  timeLabel?: string;
  conflictWith?: string[];
  overridden?: boolean;
  alternation?: Alternation;
};

/** One entry of a block's right-click menu. */
export type BlockAction = {
  label: string;
  /** The block already is what this action sets. */
  current?: boolean;
  run: () => void;
};

export type DayColumn = {
  label: string;
  date?: string;
  isToday?: boolean;
  calendarNotice?: string;
  noClasses?: boolean;
  blocks: Block[];
};

/**
 * A shared 演習・実験 block as its two courses, exercise on the left, so
 * each half names and opens its own course. Other blocks stay whole.
 */
function blockHalves(
  block: Block,
): { block: Block; side?: "left" | "right" }[] {
  const pair = block.alternation?.pair;
  if (pair?.length !== 2) return [{ block }];
  return pair.map((course, index) => ({
    block: {
      ...block,
      key: `${block.key}:${course.courseId}`,
      id: course.courseId,
      title: course.title,
      subtitle: course.code,
      room: course.room,
      color: course.color,
      startTime: course.startTime,
      endTime: course.endTime,
    },
    side: index ? "right" : "left",
  }));
}

export function WeekTimetable({
  days,
  onSelect,
  selectedId,
  now,
  onSlot,
  activeSlots,
  actionsFor,
}: {
  days: DayColumn[];
  onSelect?: (block: Block) => void;
  selectedId?: string;
  /** The shell clock (`useClock`); draws the now line in the column marked
   * today, which the caller derives from the same instant. */
  now?: string;
  /** Clicking the free area of a teaching slot (day is 1 = Monday); `append` with ⌘/Ctrl. */
  onSlot?: (day: number, from: number, to: number, append: boolean) => void;
  /** Slots to highlight, as `${day}:${from}-${to}`. */
  activeSlots?: ReadonlySet<string>;
  /** Right-click entries for a block; none leaves the native menu alone. */
  actionsFor?: (block: Block) => BlockAction[];
}) {
  const [menu, setMenu] = useState<{
    x: number;
    y: number;
    actions: BlockAction[];
  } | null>(null);
  const openMenu = (event: React.MouseEvent, block: Block) => {
    const actions = actionsFor?.(block) ?? [];
    if (!actions.length) return;
    event.preventDefault();
    setMenu({ x: event.clientX, y: event.clientY, actions });
  };
  // Every period is one row; Core places times on that axis
  // (`timetable_axis`), widened around classes outside the teaching day.
  const span = axisSpan(days.flatMap((column) => column.blocks));
  const axis = useScheduleQuery<TimetableAxis>({
    kind: "timetableAxis",
    earliest: span.earliest ?? null,
    latest: span.latest ?? null,
  }).value;
  const rows = axis ? axisRows(axis) : [];
  const nowOffset =
    now && axis ? nowPercent(axis, jstMinutes(new Date(now))) : undefined;
  return (
    <div
      className={`tt${days.some((column) => column.calendarNotice) ? " hasCalendarNotice" : ""}`}
      style={{ "--days": days.length } as React.CSSProperties}
    >
      <div className="ttGrid" role="grid" aria-label={t("Weekly timetable")}>
        <div className="ttCorner" />
        {days.map((column, index) => {
          const head = (
            <div
              className={`ttHead${column.isToday ? " today" : ""}${column.noClasses ? " noClasses" : ""}`}
              key={column.label}
            >
              <b>{t(column.label.slice(0, 3))}</b>
              {column.date && (
                <small>{column.date.slice(5).replace("-", "/")}</small>
              )}
              {column.calendarNotice && (
                <span
                  className="ttCalendarNotice"
                  title={column.calendarNotice}
                >
                  {column.calendarNotice}
                </span>
              )}
            </div>
          );
          // The right-click marks are invisible until someone is told.
          return index === 0 && actionsFor ? (
            <GuideTip
              key={column.label}
              id="timetable-marks"
              text="Right-click a class to mark it as an exam or cancelled."
              when={days.some((day) => day.blocks.length > 0)}
            >
              {head}
            </GuideTip>
          ) : (
            head
          );
        })}
        <div className="ttAxis">
          {rows.map((period) => {
            const position = period;
            return (
              <div
                className={period.isBreak ? "lunch" : ""}
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
            className={`ttCol${column.isToday ? " today" : ""}${column.noClasses ? " noClasses" : ""}`}
            key={column.label}
          >
            {onSlot &&
              dayIndex < SLOT_DAYS &&
              rows
                .filter((period) => !period.isBreak)
                .map((period) => {
                  // The selection fills the same cell a class there fills.
                  const position = period;
                  const [from, to] = period.periods as readonly [
                    number,
                    number,
                  ];
                  const active = activeSlots?.has(
                    `${dayIndex + 1}:${from}-${to}`,
                  );
                  return (
                    <button
                      type="button"
                      className={`ttSlot${active ? " active" : ""}`}
                      key={`slot-${period.label}`}
                      aria-pressed={Boolean(active)}
                      aria-label={`${t(column.label)} ${period.label}`}
                      title={t("Find courses at this time")}
                      style={{
                        top: `${position.topPercent}%`,
                        height: `${position.heightPercent}%`,
                      }}
                      onClick={(event) =>
                        onSlot(
                          dayIndex + 1,
                          from,
                          to,
                          event.metaKey || event.ctrlKey,
                        )
                      }
                    />
                  );
                })}
            {rows.map((period) => {
              const position = period;
              return (
                <i
                  className={period.isBreak ? "ttLunch" : "ttRule"}
                  key={t(period.label)}
                  style={{
                    top: `${position.topPercent}%`,
                    height: period.isBreak
                      ? `${position.heightPercent}%`
                      : undefined,
                  }}
                />
              );
            })}
            {column.blocks.flatMap(blockHalves).map(({ block, side }) => {
              // A block without readable times is left out rather than breaking the page.
              if (
                !axis ||
                !CLOCK.test(block.startTime) ||
                !CLOCK.test(block.endTime)
              )
                return null;
              const position = axisPosition(
                axis,
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
                    block.cancelled ? "cancelled" : "",
                    selectedId === block.id ? "selected" : "",
                  ]
                    .filter(Boolean)
                    .join(" ")}
                  style={
                    {
                      "--course": block.color,
                      // A 2px gutter keeps the card off the row rules.
                      top: `calc(${position.topPercent}% + 2px)`,
                      height: `calc(${position.heightPercent}% - 4px)`,
                      ...(side === "left" && { right: "calc(50% + 1px)" }),
                      ...(side === "right" && { left: "calc(50% + 1px)" }),
                    } as React.CSSProperties
                  }
                  title={`${block.title}${block.subtitle ? ` · ${block.subtitle}` : ""} · ${block.timeLabel ?? `${block.startTime}–${block.endTime}`}${block.room ? ` · ${block.room}` : ""}${block.conflictWith?.length ? ` · ${t("Overlaps {{courses}}", { courses: block.conflictWith.join(", ") })}` : ""}`}
                  key={block.key}
                  onClick={() => onSelect?.(block)}
                  onContextMenu={(event) => openMenu(event, block)}
                >
                  <b>{block.title}</b>
                  {/* An exam keeps its code and tag right under the title,
                      so a short exam block still says what it is. */}
                  {block.kind === "exam" && (
                    <span className="ttCode">
                      {block.subtitle}
                      <span className="ttExamTag">{t("Exam")}</span>
                    </span>
                  )}
                  {block.room && <small className="ttRoom">{block.room}</small>}
                  {block.alternation && (
                    <small className="ttAlternation">
                      {alternationLabel(block.alternation)}
                    </small>
                  )}
                  {block.subtitle && block.kind !== "exam" && (
                    <span className="ttCode">{block.subtitle}</span>
                  )}
                  {block.conflictWith?.length ? (
                    <small className="ttConflict">
                      {t("Overlaps {{courses}}", {
                        courses: block.conflictWith.join(", "),
                      })}
                    </small>
                  ) : null}
                </UiButton>
              );
            })}
            {column.isToday && nowOffset !== undefined && (
              <i className="ttNow" style={{ top: `${nowOffset}%` }} />
            )}
          </div>
        ))}
      </div>
      {/* A zero-size anchor at the pointer places the menu where the user
          right-clicked. */}
      <Menu
        opened={Boolean(menu)}
        onChange={(opened) => !opened && setMenu(null)}
        position="bottom-start"
        offset={2}
        withinPortal
      >
        <Menu.Target>
          <span
            aria-hidden
            style={{
              position: "fixed",
              left: menu?.x ?? 0,
              top: menu?.y ?? 0,
              width: 0,
              height: 0,
            }}
          />
        </Menu.Target>
        <Menu.Dropdown>
          {menu?.actions.map((action) => (
            <Menu.Item
              key={action.label}
              leftSection={action.current ? "✓" : " "}
              onClick={() => {
                setMenu(null);
                action.run();
              }}
            >
              {action.label}
            </Menu.Item>
          ))}
        </Menu.Dropdown>
      </Menu>
    </div>
  );
}
import { UiButton } from "../ui";

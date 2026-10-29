import { ChevronDown, ChevronRight } from "lucide-react";
import { useState, type ReactNode } from "react";
import { t } from "../../i18n";
import { UiButton, EmptyState } from "../ui";
import type { Assignment } from "../../lib/model";
import type { AssignmentGroups } from "../../generated/persistence";

/**
 * Core's assignment groups in their fixed order. Pages supply the row; older
 * past work and hidden work stay folded until asked.
 */
export function AssignmentGroupList({
  groups,
  assignments,
  row,
  hiddenRow,
  matches = () => true,
  thisWeekLimit,
  pastLabel,
  empty,
}: {
  groups: AssignmentGroups | undefined;
  assignments: ReadonlyMap<string, Assignment>;
  row: (assignment: Assignment) => ReactNode;
  /** Shown under a folded "Hidden" toggle when given. */
  hiddenRow?: (assignment: Assignment) => ReactNode;
  matches?: (assignment: Assignment) => boolean;
  /** Rows of "This week" shown before "+N more". */
  thisWeekLimit?: number;
  pastLabel: string;
  empty: string;
}) {
  const [allThisWeek, setAllThisWeek] = useState(false);
  const [showEarlier, setShowEarlier] = useState(false);
  const [showHidden, setShowHidden] = useState(false);
  const list = (ids: string[] | undefined) =>
    (ids ?? []).flatMap((id) => {
      const assignment = assignments.get(id);
      return assignment && matches(assignment) ? [assignment] : [];
    });
  const pinned = list(groups?.pinnedIds);
  const thisWeek = list(groups?.dueThisWeekIds);
  const later = list(groups?.laterIds);
  const undated = list(groups?.undatedIds);
  const past = list(groups?.pastRecentIds);
  const earlier = list(groups?.pastEarlierIds);
  const hidden = hiddenRow ? list(groups?.hiddenIds) : [];
  const limited =
    thisWeekLimit !== undefined && !allThisWeek
      ? thisWeek.slice(0, thisWeekLimit)
      : thisWeek;
  const group = (
    name: string,
    label: string,
    items: Assignment[],
    extra?: ReactNode,
  ) =>
    items.length > 0 && (
      <div className={`dueGroup ${name}`} key={name}>
        <small>
          {label} · {items.length}
        </small>
        {(name === "thisWeek" ? limited : items).map(row)}
        {extra}
      </div>
    );
  const nothing =
    !pinned.length &&
    !thisWeek.length &&
    !later.length &&
    !undated.length &&
    !past.length &&
    !earlier.length;
  return (
    <>
      {nothing && <EmptyState compact>{empty}</EmptyState>}
      {group("pinned", t("common.pinned"), pinned)}
      {group(
        "thisWeek",
        t("common.thisWeek"),
        thisWeek,
        thisWeek.length > limited.length || allThisWeek ? (
          <UiButton
            className="textButton moreToggle"
            onClick={() => setAllThisWeek((value) => !value)}
          >
            {allThisWeek
              ? t("common.showLess")
              : t("assignments.countMore", {
                  count: thisWeek.length - limited.length,
                })}
          </UiButton>
        ) : undefined,
      )}
      {group("later", t("common.later"), later)}
      {group("undated", t("common.noDeadline"), undated)}
      {group("past", pastLabel, past)}
      {earlier.length > 0 && (
        <div className="dueGroup earlier">
          <UiButton
            className="textButton moreToggle"
            aria-expanded={showEarlier}
            onClick={() => setShowEarlier((value) => !value)}
          >
            {showEarlier
              ? t("common.showLess")
              : t("assignments.countEarlierBackToThe", {
                  count: earlier.length,
                })}
          </UiButton>
          {showEarlier && earlier.map(row)}
        </div>
      )}
      {hidden.length > 0 && hiddenRow && (
        <div className="dueGroup hidden">
          <UiButton
            className="textButton moreToggle"
            aria-expanded={showHidden}
            onClick={() => setShowHidden((value) => !value)}
          >
            {showHidden ? (
              <ChevronDown size={14} strokeWidth={1.75} aria-hidden />
            ) : (
              <ChevronRight size={14} strokeWidth={1.75} aria-hidden />
            )}{" "}
            {t("assignments.hiddenAssignments")} · {hidden.length}
          </UiButton>
          {showHidden && hidden.map(hiddenRow)}
        </div>
      )}
    </>
  );
}

import { Badge, Button, Checkbox, Modal } from "@mantine/core";
import { useState, type CSSProperties, type ReactNode } from "react";
import { t } from "../../i18n";
import { useSubmissionGuard } from "../../components/use-submission-guard";
import type { MergeDiff } from "../../lib/lms-merge";

/** Colours shared by the two sides of one conflict. */
const CONFLICT_COLORS = [
  "#e8590c",
  "#7048e8",
  "#0c8599",
  "#c2255c",
  "#2f9e44",
  "#1971c2",
];

const termLabel = (term: string) => term.replace("-", " ");
const DAYS = ["月", "火", "水", "木", "金", "土", "日"];
const gridLabel = (
  meetings: { day: number; periods: [number, number] }[] = [],
) =>
  meetings
    .map((item) => `${DAYS[item.day - 1]}${item.periods[0]}-${item.periods[1]}`)
    .join(" ") || "—";

function Column({
  title,
  count,
  children,
}: {
  title: string;
  count: number;
  children: ReactNode;
}) {
  return (
    <section className="mergeColumn">
      <header>
        <b>{title}</b>
        <span>{count}</span>
      </header>
      <div className="mergeList">{children}</div>
      {!count && <small className="mergeEmpty">{t("None")}</small>}
    </section>
  );
}

/**
 * Review of an LMS pull: courses on both sides, only on this device, and only
 * on LMS. Conflicting pairs (same slot, or same course in another quarter)
 * share a highlight colour. Nothing changes until Apply.
 */
export function LmsMergeDialog({
  diff,
  close,
  apply,
}: {
  diff: MergeDiff;
  close: () => Promise<unknown> | void;
  apply: (
    add: Set<string>,
    keep: Set<string>,
    retime: Set<string>,
  ) => Promise<unknown> | void;
}) {
  const submission = useSubmissionGuard();
  // LMS-only courses are added and local-only ones kept unless unchecked.
  const [add, setAdd] = useState(
    () => new Set(diff.lmsOnly.map((entry) => entry.key)),
  );
  const [keep, setKeep] = useState(
    () => new Set(diff.localOnly.map((entry) => entry.key)),
  );
  // Courses whose LMS timetable slot differs take the LMS time unless unchecked.
  const [retime, setRetime] = useState(
    () =>
      new Set(
        diff.common
          .filter((pair) => pair.timeDiffers)
          .map((pair) => `local:${pair.local.id}`),
      ),
  );
  const multiTerm = diff.terms.length > 1;
  const toggle = (
    set: Set<string>,
    update: (next: Set<string>) => void,
    key: string,
  ) => {
    const next = new Set(set);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    update(next);
  };
  const conflictStyle = (key: string) => {
    const conflict = diff.conflicts.get(key);
    return conflict
      ? ({
          "--conflict":
            CONFLICT_COLORS[conflict.group % CONFLICT_COLORS.length],
        } as CSSProperties)
      : undefined;
  };
  const conflictBadge = (key: string) => {
    const conflict = diff.conflicts.get(key);
    if (!conflict) return null;
    return (
      <Badge size="xs" variant="filled" className="mergeConflictBadge">
        {conflict.reason === "slot" ? t("Same slot") : t("Other quarter")}
      </Badge>
    );
  };
  return (
    <Modal
      opened
      onClose={() => void submission.run(close)}
      closeOnClickOutside={!submission.busy}
      closeOnEscape={!submission.busy}
      size="min(1100px, 94vw)"
      title={`${t("Pull from LMS")} · ${diff.terms.length === 1 ? termLabel(diff.terms[0]) : t("All quarters")}`}
      classNames={{ body: "mergeBody" }}
    >
      <div className="mergeGrid">
        <Column title={t("Common")} count={diff.common.length}>
          {diff.common.map((pair) =>
            pair.timeDiffers ? (
              <label
                className="mergeItem retime"
                key={`${pair.termId}:${pair.local.id}`}
              >
                <Checkbox
                  size="xs"
                  checked={retime.has(`local:${pair.local.id}`)}
                  onChange={() =>
                    toggle(retime, setRetime, `local:${pair.local.id}`)
                  }
                  aria-label={t("Use LMS time")}
                />
                <span>
                  <b>{pair.local.title}</b>
                  <small>
                    {t("LMS time")} {gridLabel(pair.lms.gridMeetings)}
                    {multiTerm && ` · ${termLabel(pair.termId)}`}
                  </small>
                </span>
                <Badge size="xs" variant="light" color="orange">
                  {t("Time differs")}
                </Badge>
              </label>
            ) : (
              <div
                className="mergeItem common"
                key={`${pair.termId}:${pair.local.id}`}
              >
                <b>{pair.local.title}</b>
                <small>
                  {pair.local.code}
                  {multiTerm && ` · ${termLabel(pair.termId)}`}
                </small>
              </div>
            ),
          )}
        </Column>
        <div className="mergeSides">
          <Column
            title={t("Only on this device")}
            count={diff.localOnly.length}
          >
            {diff.localOnly.map((entry) => (
              <label
                className={`mergeItem${diff.conflicts.has(entry.key) ? " conflict" : ""}`}
                style={conflictStyle(entry.key)}
                key={entry.key}
              >
                <Checkbox
                  size="xs"
                  checked={keep.has(entry.key)}
                  onChange={() => toggle(keep, setKeep, entry.key)}
                  aria-label={t("Keep")}
                />
                <span>
                  <b>{entry.course.title}</b>
                  <small>
                    {entry.course.code || "—"}
                    {multiTerm && ` · ${termLabel(entry.termId)}`}
                  </small>
                </span>
                {conflictBadge(entry.key)}
              </label>
            ))}
          </Column>
          <Column title={t("Only on LMS")} count={diff.lmsOnly.length}>
            {diff.lmsOnly.map((entry) => (
              <label
                className={`mergeItem${diff.conflicts.has(entry.key) ? " conflict" : ""}`}
                style={conflictStyle(entry.key)}
                key={entry.key}
              >
                <Checkbox
                  size="xs"
                  checked={add.has(entry.key)}
                  onChange={() => toggle(add, setAdd, entry.key)}
                  aria-label={t("Add")}
                />
                <span>
                  <b>{entry.title}</b>
                  <small>
                    {entry.code}
                    {entry.gridMeetings &&
                      ` · ${gridLabel(entry.gridMeetings)}`}
                    {multiTerm && ` · ${termLabel(entry.termId)}`}
                    {!entry.offering && ` · ${t("no syllabus")}`}
                  </small>
                </span>
                {conflictBadge(entry.key)}
              </label>
            ))}
          </Column>
        </div>
      </div>
      {submission.error && <div role="alert">{submission.error}</div>}
      <footer className="mergeFoot">
        <small>
          {t(
            "Checked: keep / add. LMS membership is not proof of registration.",
          )}
        </small>
        <Button
          variant="default"
          disabled={submission.busy}
          onClick={() => void submission.run(close)}
        >
          {t("Cancel")}
        </Button>
        <Button
          disabled={submission.busy}
          onClick={() => void submission.run(() => apply(add, keep, retime))}
        >
          {t("Apply")}
        </Button>
      </footer>
    </Modal>
  );
}

import { Button, Popover } from "@mantine/core";
import { t } from "../../i18n";
import type { GpaSummary } from "../../core/model";

const SCOPES: { scope: GpaSummary["scope"]; label: string }[] = [
  { scope: "all", label: "All" },
  { scope: "liberal-arts", label: "Liberal arts" },
  { scope: "major", label: "Major" },
];
const fixed = (value: number | undefined, digits: number) =>
  value === undefined ? "—" : value.toFixed(digits);

/**
 * GPA, GPT and earned credits from 教務Web, plus the credits on this
 * quarter's timetable. Opens the per-group breakdown and a refresh.
 */
export function AcademicSummary({
  summaries,
  termCredits,
  refreshing,
  refresh,
  openOfficial,
}: {
  summaries: GpaSummary[];
  termCredits: number;
  refreshing: boolean;
  refresh?: () => void;
  openOfficial: () => void;
}) {
  const all = summaries.find((item) => item.scope === "all" && !item.deletedAt);
  return (
    <Popover position="bottom-end" withinPortal shadow="md">
      <Popover.Target>
        <button type="button" className="academicChip" title={t("Grades from 教務Web")}>
          {all ? (
            <>
              <span>
                GPA <b>{fixed(all.gpa, 2)}</b>
              </span>
              <span>
                GPT <b>{fixed(all.gpt, 1)}</b>
              </span>
              <span>
                {t("Earned")} <b>{all.earnedCredits}</b>
              </span>
            </>
          ) : (
            <span>GPA —</span>
          )}
          <span className="academicChipTerm">
            {t("This quarter")} <b>{termCredits}</b>
          </span>
        </button>
      </Popover.Target>
      <Popover.Dropdown className="academicPopover">
        <table>
          <thead>
            <tr>
              <th />
              <th>GPA</th>
              <th>GPT</th>
              <th>{t("Credits")}</th>
            </tr>
          </thead>
          <tbody>
            {SCOPES.map(({ scope, label }) => {
              const row = summaries.find((item) => item.scope === scope && !item.deletedAt);
              return (
                <tr key={scope}>
                  <th>{t(label)}</th>
                  <td>{fixed(row?.gpa, 2)}</td>
                  <td>{fixed(row?.gpt, 1)}</td>
                  <td>{row?.earnedCredits ?? "—"}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
        <footer>
          <small>
            {all ? new Date(all.fetchedAt).toLocaleString() : t("Not fetched yet")}
          </small>
          <Button size="compact-xs" variant="subtle" onClick={openOfficial}>
            教務Web ↗
          </Button>
          {refresh && (
            <Button size="compact-xs" loading={refreshing} onClick={refresh}>
              {t("Refresh")}
            </Button>
          )}
        </footer>
      </Popover.Dropdown>
    </Popover>
  );
}

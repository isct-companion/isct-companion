import { Button, Paper, Popover, Text } from "../../components/ui";
import { useState } from "react";
import { SyncRing, progressLabel } from "../../components/PullRing";
import type { PageProgress } from "../../platform/sync";
import i18n, { t, intlLocale } from "../../i18n";
import { formatTimestamp } from "../../lib/civil-date";
import type { GpaSummary } from "../../lib/model";
import type { SchoolPageAvailability } from "../../generated/persistence";

const SCOPES: { scope: GpaSummary["scope"]; label: string }[] = [
  { scope: "all", label: "All" },
  { scope: "liberal-arts", label: "Liberal arts" },
  { scope: "major", label: "Major" },
];
const fixed = (value: number | undefined, digits: number) =>
  value === undefined ? "—" : value.toFixed(digits);

/** Quarter credits stay visible; grades are rendered only after clicking Details. */
export function AcademicSummary({
  summaries,
  termCredits,
  progress = null,
  availability,
  refresh,
  openOfficial,
}: {
  summaries: GpaSummary[];
  termCredits: number;
  progress?: PageProgress;
  availability?: SchoolPageAvailability;
  refresh?: () => void;
  openOfficial: () => void;
}) {
  const all = summaries.find((item) => item.scope === "all" && !item.deletedAt);
  const unavailable = availability?.state === "unavailable";
  const availabilityChecked = availability
    ? formatTimestamp(availability.checkedAt, intlLocale(i18n.language))
    : undefined;
  const [opened, setOpened] = useState(false);
  return (
    <div className="academicSummaryTrigger">
      <span className="academicQuarterLabel">
        {t("This term")} <b>{termCredits}</b> {t("credits")}
      </span>
      <Popover
        position="bottom-end"
        withinPortal
        shadow="md"
        width={300}
        opened={opened}
        onChange={setOpened}
      >
        <Popover.Target>
          <Button
            size="xs"
            variant="subtle"
            className="academicDetailsTrigger"
            onClick={() => setOpened((value) => !value)}
          >
            {t("Details")}
          </Button>
        </Popover.Target>
        <Popover.Dropdown p={0}>
          {opened && (
            <Paper
              component="section"
              className="academicPanel"
              radius="md"
              aria-label={t("Academic summary")}
            >
              <header>
                <h2>{t("Academic summary")}</h2>
              </header>
              <div className="academicTermCredits">
                <span>{t("This quarter")}</span>
                <b>
                  {termCredits} <small>{t("credits")}</small>
                </b>
              </div>
              <Text size="xs" c="dimmed">
                {t(
                  "Credits for multi-quarter courses are split equally across quarters.",
                )}
              </Text>
              <div className="academicGradeDetails">
                {unavailable && (
                  <div className="academicAvailability" role="status">
                    <b>
                      {t("This GPA page is not provided for this account.")}
                    </b>
                    <small>
                      {all
                        ? t(
                            "The values below were retained from an earlier successful fetch.",
                          )
                        : availability.message}
                    </small>
                  </div>
                )}
                {(!unavailable || all) && (
                  <table>
                    <caption className="srOnly">
                      {t("Grades from 教務Web")}
                    </caption>
                    <thead>
                      <tr>
                        <th scope="col">{t("Scope")}</th>
                        <th scope="col">GPA</th>
                        <th scope="col">GPT</th>
                        <th scope="col">{t("Earned")}</th>
                      </tr>
                    </thead>
                    <tbody>
                      {SCOPES.map(({ scope, label }) => {
                        const row = summaries.find(
                          (item) => item.scope === scope && !item.deletedAt,
                        );
                        return (
                          <tr key={scope}>
                            <th scope="row">{t(label)}</th>
                            <td>{fixed(row?.gpa, 2)}</td>
                            <td>{fixed(row?.gpt, 2)}</td>
                            <td>{row?.earnedCredits ?? "—"}</td>
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                )}
                <small>
                  {all
                    ? `${unavailable ? `${t("Previous fetch")}: ` : ""}${
                        formatTimestamp(
                          all.fetchedAt,
                          intlLocale(i18n.language),
                        ) ?? t("Not fetched yet")
                      }`
                    : unavailable
                      ? `${t("Checked")}: ${availabilityChecked ?? availability.checkedAt}`
                      : t("Not fetched yet")}
                </small>
              </div>
              <footer>
                <Button
                  size="compact-xs"
                  variant="subtle"
                  onClick={openOfficial}
                >
                  教務Web ↗
                </Button>
                {refresh && (
                  <Button
                    size="compact-xs"
                    variant="default"
                    disabled={progress !== null}
                    leftSection={
                      progress !== null ? (
                        <SyncRing progress={progress} size={14} />
                      ) : undefined
                    }
                    onClick={refresh}
                  >
                    {progress !== null ? progressLabel(progress) : t("Refresh")}
                  </Button>
                )}
              </footer>
            </Paper>
          )}
        </Popover.Dropdown>
      </Popover>
    </div>
  );
}

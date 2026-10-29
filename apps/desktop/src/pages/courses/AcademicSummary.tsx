import { Button, Paper, Popover, Text } from "../../components/ui";
import { useState } from "react";
import { SyncRing, progressLabel } from "../../components/PullRing";
import type { PageProgress } from "../../platform/sync";
import i18n, { t, intlLocale, type MessageKey } from "../../i18n";
import { formatTimestamp } from "../../lib/civil-date";
import type { GpaSummary } from "../../lib/model";
import type { SchoolPageAvailability } from "../../generated/persistence";

const SCOPES: { scope: GpaSummary["scope"]; label: MessageKey }[] = [
  { scope: "all", label: "courses.all" },
  { scope: "liberal-arts", label: "courses.liberalArts" },
  { scope: "major", label: "courses.major" },
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
        {t("courses.thisTerm")} <b>{termCredits}</b> {t("common.creditsUnit")}
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
            {t("courses.details")}
          </Button>
        </Popover.Target>
        <Popover.Dropdown p={0}>
          {opened && (
            <Paper
              component="section"
              className="academicPanel"
              radius="md"
              aria-label={t("courses.academicSummary")}
            >
              <header>
                <h2>{t("courses.academicSummary")}</h2>
              </header>
              <div className="academicTermCredits">
                <span>{t("courses.thisQuarter")}</span>
                <b>
                  {termCredits} <small>{t("common.creditsUnit")}</small>
                </b>
              </div>
              <Text size="xs" c="dimmed">
                {t("courses.creditsForMultiQuarterCourses")}
              </Text>
              <div className="academicGradeDetails">
                {unavailable && (
                  <div className="academicAvailability" role="status">
                    <b>{t("courses.thisGPAPageIsNot")}</b>
                    <small>
                      {all
                        ? t("courses.theValuesBelowWereRetained")
                        : availability.message}
                    </small>
                  </div>
                )}
                {(!unavailable || all) && (
                  <table>
                    <caption className="srOnly">
                      {t("courses.gradesFromWeb")}
                    </caption>
                    <thead>
                      <tr>
                        <th scope="col">{t("courses.scope")}</th>
                        <th scope="col">GPA</th>
                        <th scope="col">GPT</th>
                        <th scope="col">{t("courses.earned")}</th>
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
                    ? `${unavailable ? `${t("courses.previousFetch")}: ` : ""}${
                        formatTimestamp(
                          all.fetchedAt,
                          intlLocale(i18n.language),
                        ) ?? t("courses.notFetchedYet")
                      }`
                    : unavailable
                      ? `${t("courses.checked")}: ${availabilityChecked ?? availability.checkedAt}`
                      : t("courses.notFetchedYet")}
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
                    {progress !== null
                      ? progressLabel(progress)
                      : t("courses.refresh")}
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

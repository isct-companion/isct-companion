import { useState } from "react";
import { Button, Menu } from "../../components/ui";
import { notify } from "../../components/notifications/notify";
import type { CalendarExportScope } from "../../generated/persistence";
import { t } from "../../i18n";
import type { TermId } from "../../lib/model";
import { scheduleQuery } from "../../platform/core-query";
import { saveExportFile } from "../../platform/save-export";
import { wallNow } from "../../platform/use-clock";

export function TimetableExportButton({
  term,
  available,
}: {
  term: TermId;
  available: boolean;
}) {
  const [exporting, setExporting] = useState(false);
  const title = t("common.exportICS");
  const canExport = available && !exporting;

  async function exportQuarter(scope: CalendarExportScope) {
    if (!canExport) return;
    setExporting(true);
    const suffix = scope === "courses-and-events" ? "-with-events" : "";
    try {
      await saveExportFile(
        `IsctCompanion-${term}${suffix}.ics`,
        () =>
          scheduleQuery<string>({
            kind: "quarterIcs",
            term,
            scope,
            generatedAt: new Date(wallNow()).toISOString(),
          }),
        notify.info,
      );
    } finally {
      setExporting(false);
    }
  }

  return (
    <Menu position="bottom-end">
      <Menu.Target>
        <Button
          size="xs"
          variant="default"
          disabled={!available}
          aria-busy={exporting}
        >
          {title}
        </Button>
      </Menu.Target>
      <Menu.Dropdown>
        <Menu.Label>
          {title} · {term.replace("-", " ")}
        </Menu.Label>
        <Menu.Item
          disabled={!canExport}
          onClick={() => void exportQuarter("courses")}
        >
          {t("common.coursesOnly")}
        </Menu.Item>
        <Menu.Item
          disabled={!canExport}
          onClick={() => void exportQuarter("courses-and-events")}
        >
          {t("common.coursesWeeklyEvents")}
        </Menu.Item>
      </Menu.Dropdown>
    </Menu>
  );
}

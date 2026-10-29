import { useEffect, useRef, useState } from "react";
import { Switch } from "../../components/ui";
import { useVirtualizer } from "@tanstack/react-virtual";
import { t, intlLocale } from "../../i18n";
import { formatTimestamp, tokyoDate } from "../../lib/civil-date";
import type { Workspace } from "../../lib/model";
import { desktopBridge, saveExportFile } from "../../app/native";
import { errorText } from "../../app/shared";
import type { ApplicationLog } from "../../platform/desktop-bridge";
import { SettingsHeader } from "../../components/forms";
import { UiButton } from "../../components/ui";

const LOG_LIMIT = 2000;

export function DiagnosticsSettings({
  data,
  notify,
}: {
  data: Workspace;
  notify: (message: string) => void;
}) {
  const [logs, setLogs] = useState<ApplicationLog[]>([]);
  const [error, setError] = useState(false);
  const [follow, setFollow] = useState(true);
  const viewport = useRef<HTMLDivElement>(null);
  const automaticScroll = useRef(false);
  useEffect(() => {
    if (!desktopBridge?.applicationLogs) return;
    let disposed = false;
    let pending = false;
    let cursor: number | undefined;
    const read = async () => {
      if (pending) return;
      pending = true;
      try {
        const snapshot = await desktopBridge!.applicationLogs!(cursor);
        if (disposed) return;
        cursor = snapshot.cursor;
        setError(false);
        if (snapshot.entries.length || snapshot.dropped)
          setLogs((current) =>
            [...(snapshot.dropped ? [] : current), ...snapshot.entries].slice(
              -LOG_LIMIT,
            ),
          );
      } catch {
        if (!disposed) setError(true);
      } finally {
        pending = false;
      }
    };
    void read();
    const timer = window.setInterval(() => void read(), 1000);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, []);
  const rows = useVirtualizer({
    count: logs.length,
    getScrollElement: () => viewport.current,
    getItemKey: (index) => logs[index].id,
    estimateSize: () => 48,
    initialRect: { width: 800, height: 420 },
    overscan: 5,
    useFlushSync: false,
  });
  useEffect(() => {
    if (!follow || !logs.length) return;
    automaticScroll.current = true;
    let release = 0;
    const frame = requestAnimationFrame(() => {
      rows.scrollToIndex(logs.length - 1, { align: "end" });
      release = requestAnimationFrame(() => {
        automaticScroll.current = false;
      });
    });
    return () => {
      cancelAnimationFrame(frame);
      cancelAnimationFrame(release);
      automaticScroll.current = false;
    };
  }, [follow, logs, rows]);
  const saveLogs = () =>
    void saveExportFile(
      `IsctCompanion-logs-${tokyoDate(new Date())}.txt`,
      () => desktopBridge!.exportApplicationLogs!(),
      notify,
    );
  const copyLogs = () =>
    void desktopBridge!.copyApplicationLogs!()
      .then(() => notify(t("settings.credentialImport.copied")))
      .catch((reason: unknown) =>
        notify(errorText(reason) ?? t("settings.otp.copyFailed")),
      );
  const canExport = Boolean(
    desktopBridge?.exportApplicationLogs && desktopBridge.copyApplicationLogs,
  );
  return (
    <>
      <SettingsHeader
        title="settings.main.applicationLogs"
        description="settings.diagnostics.readOnlyLogsFromThis"
      />
      <div className="diagnosticToolbar">
        <span className="muted">
          {t("settings.diagnostics.countLogEntries", { count: logs.length })}
        </span>
        <div className="diagnosticActions">
          <Switch
            label={t("settings.diagnostics.followLatest")}
            checked={follow}
            onCheckedChange={setFollow}
          />
          {canExport && (
            <>
              <UiButton className="textButton" onClick={saveLogs}>
                {t("settings.main.saveToFile")}
              </UiButton>
              <UiButton className="textButton" onClick={copyLogs}>
                {t("settings.schoolAccounts.copy")}
              </UiButton>
            </>
          )}
        </div>
      </div>
      {error && (
        <p role="status" className="danger">
          {t("settings.diagnostics.applicationLogsAreUnavailable")}
        </p>
      )}
      <div
        className="diagnosticLogs"
        ref={viewport}
        aria-label={t("settings.main.applicationLogs")}
        tabIndex={0}
        onScroll={(event) => {
          const node = event.currentTarget;
          if (
            !automaticScroll.current &&
            node.scrollHeight - node.scrollTop - node.clientHeight > 48
          )
            setFollow(false);
        }}
        onWheel={(event) => {
          if (event.deltaY < 0) setFollow(false);
        }}
        onKeyDown={(event) => {
          if (["ArrowUp", "PageUp", "Home"].includes(event.key))
            setFollow(false);
        }}
      >
        {!logs.length && (
          <p className="muted">
            {t(
              desktopBridge
                ? "settings.diagnostics.noLogEntriesYet"
                : "settings.diagnostics.appLogsAreAvailableIn",
            )}
          </p>
        )}
        <div
          style={{
            height: rows.getTotalSize(),
            position: "relative",
            width: "100%",
          }}
        >
          {rows.getVirtualItems().map((row) => {
            const entry = logs[row.index];
            return (
              <div
                key={row.key}
                ref={rows.measureElement}
                data-index={row.index}
                className="diagnosticLogRow"
                data-level={entry.level}
                style={{
                  position: "absolute",
                  top: 0,
                  left: 0,
                  width: "100%",
                  transform: `translateY(${row.start}px)`,
                }}
              >
                <span className="diagnosticLogMeta">
                  {formatTimestamp(
                    entry.at,
                    intlLocale(data.settings.locale),
                  ) ?? entry.at}{" "}
                  · {entry.level} · {entry.target}
                </span>
                <span>{entry.message}</span>
              </div>
            );
          })}
        </div>
      </div>
    </>
  );
}

import { Checkbox, Radio } from "../../components/ui";
import { invoke } from "../../platform/native-error";
import { useEffect, useState } from "react";
import { intlLocale, t, type MessageKey } from "../../i18n";
import { errorText } from "../../app/shared";
import { desktopBridge } from "../../app/native";
import type { CloudSyncProvider } from "../../platform/desktop-bridge";
import { workspaceActions } from "../../store/workspace";
import { UiButton } from "../../components/ui";
import { Row, SettingsGroup } from "../../components/forms";

/** `app_core::sync_engine::ResetScope`; Core decides what each one removes. */
type ResetScope =
  | { kind: "school-caches"; sign_out_web: boolean; delete_downloads: boolean }
  | { kind: "everything" };
type ResetReport = { removedRows: number; freedBytes: number };

type Kind = ResetScope["kind"];
type AfterReset = "restore" | "new-device";

/** What every reset removes; the item keys live under `settings.reset.item`. */
const ITEM_KEYS = {
  schoolData: "settings.reset.item.schoolData",
  catalog: "settings.reset.item.catalog",
  references: "settings.reset.item.references",
  syncRecords: "settings.reset.item.syncRecords",
  lmsToken: "settings.reset.item.lmsToken",
  webSession: "settings.reset.item.webSession",
  downloads: "settings.reset.item.downloads",
  userData: "settings.reset.item.userData",
  settings: "settings.reset.item.settings",
  logins: "settings.reset.item.logins",
  notes: "settings.reset.item.notes",
  courseFolder: "settings.reset.item.courseFolder",
  cloud: "settings.reset.item.cloud",
  logs: "settings.reset.item.logs",
} satisfies Record<string, MessageKey>;
type ResetItem = keyof typeof ITEM_KEYS;
const only = (on: boolean, item: ResetItem): ResetItem[] => (on ? [item] : []);
const SCHOOL_CACHES: ResetItem[] = [
  "schoolData",
  "catalog",
  "references",
  "syncRecords",
  "lmsToken",
];

function formatSize(bytes: number, locale: string) {
  const [value, unit]: [number, string] =
    bytes >= 1e9
      ? [bytes / 1e9, "gigabyte"]
      : bytes >= 1e6
        ? [bytes / 1e6, "megabyte"]
        : [bytes / 1e3, "kilobyte"];
  return new Intl.NumberFormat(intlLocale(locale), {
    style: "unit",
    unit,
    maximumFractionDigits: 1,
  }).format(value);
}

function Items({ title, items }: { title: MessageKey; items: ResetItem[] }) {
  return (
    <div>
      <b>{t(title)}</b>
      <ul>
        {items.map((item) => (
          <li key={item}>{t(ITEM_KEYS[item])}</li>
        ))}
      </ul>
    </div>
  );
}

/**
 * Settings → Reset (`docs/data-ownership.md`): clear the school caches and read them
 * again, or return the device to its first-launch state. Each opens a
 * confirmation panel that lists what goes, what stays and about how much
 * space is freed; a full reset needs one more confirmation.
 */
export function ResetSettings({
  locale,
  cloudProviders,
  notify,
}: {
  locale: string;
  /** Providers with cloud sync on; a full reset asks what happens to them. */
  cloudProviders: CloudSyncProvider[];
  notify: (message: string) => void;
}) {
  const [open, setOpen] = useState<Kind>();
  const [signOutWeb, setSignOutWeb] = useState(false);
  const [deleteDownloads, setDeleteDownloads] = useState(false);
  const [acknowledged, setAcknowledged] = useState(false);
  const [after, setAfter] = useState<AfterReset>("restore");
  /** `undefined` while estimating, `null` when no estimate is available. */
  const [preview, setPreview] = useState<ResetReport | null>();
  const [resetting, setResetting] = useState(false);
  const cloudOn = cloudProviders.length > 0;

  const scopeOf = (kind: Kind): ResetScope =>
    kind === "everything"
      ? { kind }
      : {
          kind,
          sign_out_web: signOutWeb,
          delete_downloads: deleteDownloads,
        };

  // Signing out frees no space, so only the downloads option re-estimates.
  useEffect(() => {
    if (!open) return;
    let current = true;
    setPreview(undefined);
    const scope: ResetScope =
      open === "everything"
        ? { kind: open }
        : {
            kind: open,
            sign_out_web: false,
            delete_downloads: deleteDownloads,
          };
    invoke<ResetReport>("reset_caches_preview", { scope })
      .then((report) => current && setPreview(report))
      .catch(() => current && setPreview(null));
    return () => {
      current = false;
    };
  }, [open, deleteDownloads]);

  const show = (kind: Kind) => {
    setOpen(kind);
    setSignOutWeb(false);
    setDeleteDownloads(false);
    setAcknowledged(false);
    setAfter("restore");
  };

  const restoreFromCloud = async () => {
    for (const provider of cloudProviders)
      await desktopBridge?.setCloudSyncProvider?.(provider, true);
    await desktopBridge?.syncCloudNow?.("pull");
    await workspaceActions.refreshSchool();
  };

  const reset = async () => {
    if (!open) return;
    const kind = open;
    setResetting(true);
    try {
      await invoke<ResetReport>("reset_caches", { scope: scopeOf(kind) });
      await workspaceActions.refreshSchool();
      setOpen(undefined);
      if (kind === "everything" && cloudOn && after === "restore") {
        try {
          await restoreFromCloud();
        } catch {
          notify(t("settings.reset.full.restoreFailed"));
          return;
        }
      }
      notify(
        t(
          kind === "everything"
            ? "settings.reset.full.done"
            : "settings.reset.caches.done",
        ),
      );
    } catch (error) {
      notify(errorText(error) ?? t("settings.reset.failed"));
    } finally {
      setResetting(false);
    }
  };

  const deletes: ResetItem[] =
    open === "everything"
      ? [...SCHOOL_CACHES, "webSession", "userData", "settings", "logins"]
      : [
          ...SCHOOL_CACHES,
          ...only(signOutWeb, "webSession"),
          ...only(deleteDownloads, "downloads"),
        ];
  const keeps: ResetItem[] =
    open === "everything"
      ? ["courseFolder", "cloud", "logs"]
      : [
          "userData",
          "settings",
          "logins",
          "notes",
          ...only(!signOutWeb, "webSession"),
          ...only(!deleteDownloads, "downloads"),
        ];

  const opener = (kind: Kind, label: MessageKey) => (
    <div className="actions">
      <UiButton
        className={`button small${kind === "everything" ? " danger" : ""}`}
        disabled={resetting || open !== undefined}
        onClick={() => show(kind)}
      >
        {t(resetting && open === kind ? "settings.reset.resetting" : label)}
      </UiButton>
    </div>
  );

  const panel = open && (
    <div
      className="alert"
      role="group"
      aria-label={t(
        open === "everything"
          ? "settings.reset.full.title"
          : "settings.reset.caches.title",
      )}
    >
      <Items title="settings.reset.deletes" items={deletes} />
      <Items title="settings.reset.keeps" items={keeps} />
      <div>
        <b>{t("settings.reset.freed")}</b>{" "}
        <span>
          {preview === undefined
            ? t("settings.reset.estimating")
            : preview === null
              ? "–"
              : formatSize(preview.freedBytes, locale)}
        </span>
      </div>
      {open === "school-caches" && (
        <>
          <Checkbox
            checked={signOutWeb}
            disabled={resetting}
            label={t("settings.reset.caches.signOut")}
            description={t("settings.reset.caches.signOutHint")}
            onCheckedChange={(checked) => setSignOutWeb(checked === true)}
          />
          <Checkbox
            checked={deleteDownloads}
            disabled={resetting}
            label={t("settings.reset.caches.downloads")}
            description={t("settings.reset.caches.downloadsHint")}
            onCheckedChange={(checked) => setDeleteDownloads(checked === true)}
          />
        </>
      )}
      {open === "everything" && cloudOn && (
        <Radio.Group
          label={t("settings.reset.full.cloudQuestion")}
          value={after}
          onChange={(value) => setAfter(value as AfterReset)}
        >
          <Radio
            value="restore"
            disabled={resetting}
            label={t("settings.reset.full.restore")}
            description={t("settings.reset.full.restoreHint")}
          />
          <Radio
            value="new-device"
            disabled={resetting}
            label={t("settings.reset.full.newDevice")}
            description={t("settings.reset.full.newDeviceHint")}
          />
        </Radio.Group>
      )}
      {open === "everything" && (
        <Checkbox
          checked={acknowledged}
          disabled={resetting}
          label={t("settings.reset.full.acknowledge")}
          onCheckedChange={(checked) => setAcknowledged(checked === true)}
        />
      )}
      <div className="actions">
        <UiButton
          className="button small danger"
          disabled={resetting || (open === "everything" && !acknowledged)}
          onClick={() => void reset()}
        >
          {t(
            resetting
              ? "settings.reset.resetting"
              : open === "everything"
                ? "settings.reset.full.confirm"
                : "settings.reset.caches.confirm",
          )}
        </UiButton>
        <UiButton
          className="textButton"
          disabled={resetting}
          onClick={() => setOpen(undefined)}
        >
          {t("settings.reset.cancel")}
        </UiButton>
      </div>
    </div>
  );

  return (
    <SettingsGroup title="settings.reset.group" tone="danger">
      <Row
        id="clear-caches"
        group
        title="settings.reset.caches.title"
        hint={t("settings.reset.caches.hint")}
      >
        {opener("school-caches", "settings.reset.caches.open")}
      </Row>
      {open === "school-caches" && panel}
      <Row
        id="full-reset"
        group
        title="settings.reset.full.title"
        hint={t("settings.reset.full.hint")}
      >
        {opener("everything", "settings.reset.full.open")}
      </Row>
      {open === "everything" && panel}
    </SettingsGroup>
  );
}

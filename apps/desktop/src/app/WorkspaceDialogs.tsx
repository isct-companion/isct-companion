import { useEffect, useRef, useState } from "react";
import { dataNoticeDialog } from "../components/DataNotice";
import { useWelcome, WelcomeSheet } from "../components/guide";
import { ConfirmDialogs, type NoticeDialog } from "../components/confirm";
import { classGroupDialog } from "../pages/courses/ClassGroupPicker";
import { notify } from "../components/notifications/notify";
import { useDesktopUpdates } from "./useDesktopUpdates";
import { useSync } from "../store/sync";
import { workspaceActions } from "../store/workspace";
import { scheduleQuery } from "../platform/core-query";
import { openExternalUrl } from "../platform/external-url";
import { tokyoDate } from "../lib/civil-date";
import { wallNow } from "../platform/use-clock";
import type { ClassGroupChoice, Workspace } from "../lib/model";
import type {
  AnnouncementsView,
  AnnouncementView,
} from "../generated/persistence";
import { t } from "../i18n";

function announcementDialog(item: AnnouncementView): NoticeDialog {
  return {
    id: `announcement:${item.id}`,
    announcementId: item.id,
    title: item.title,
    message: item.body,
    choices: [
      ...(item.link ? [{ id: "details", label: item.link.label }] : []),
      { id: "acknowledge", label: t("announcements.acknowledge") },
    ],
    onChoice: (choice) => {
      if (!choice) return;
      void workspaceActions
        .mutate({ kind: "answerAnnouncement", id: item.id })
        .catch(notify.error);
      if (choice === "details" && item.link)
        void openExternalUrl(item.link.url).catch(notify.error);
    },
  };
}

export function WorkspaceDialogs({
  settings,
  update,
  readOnly,
  connected,
  openAccounts,
  openAgents,
  announcements,
  readingAnnouncements,
}: {
  settings: Workspace["settings"];
  update: (edit: (draft: Workspace) => Workspace) => void;
  readOnly: boolean;
  connected: boolean;
  openAccounts: () => void;
  openAgents: () => void;
  announcements?: AnnouncementsView;
  readingAnnouncements: boolean;
}) {
  const needsDataNotice = !readOnly && !settings.dataNoticeAcknowledgedAt;
  const [needsWelcome, finishWelcome] = useWelcome();
  const offer = useDesktopUpdates(!readOnly, settings.automaticAppUpdates);
  const [group, setGroup] = useState<ClassGroupChoice>();
  const sources = useSync().sources;
  const kyomu =
    sources.find((source) => source.source === "kyomu")?.lastUpdated ?? null;
  const lastKyomu = useRef<string | null | undefined>(undefined);
  useEffect(() => {
    if (readOnly || !sources.length) return;
    const previous = lastKyomu.current;
    lastKyomu.current = kyomu;
    if (previous === undefined || kyomu === null || kyomu === previous) return;
    let cancelled = false;
    void scheduleQuery<ClassGroupChoice[]>({
      kind: "classGroups",
      today: tokyoDate(new Date(wallNow())),
    })
      .then((choices) => {
        if (!cancelled) setGroup(choices.find((choice) => choice.needsChoice));
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [readOnly, kyomu, sources.length]);

  const automatic: NoticeDialog[] = needsDataNotice
    ? [
        dataNoticeDialog(() =>
          update((draft) => {
            draft.settings.dataNoticeAcknowledgedAt = new Date().toISOString();
            return draft;
          }),
        ),
      ]
    : needsWelcome
      ? []
      : [
          ...(group
            ? [
                classGroupDialog(
                  group,
                  (value) => {
                    setGroup(undefined);
                    void workspaceActions
                      .setClassGroup(group.semester, value)
                      .catch(notify.error);
                  },
                  () => setGroup(undefined),
                ),
              ]
            : []),
          ...(offer && settings.appUpdateChecks !== false ? [offer] : []),
          ...(readingAnnouncements
            ? []
            : (announcements?.pending ?? []).map(announcementDialog)),
        ];

  return (
    <>
      {!needsDataNotice && needsWelcome && (
        <WelcomeSheet
          settings={settings}
          update={update}
          connected={connected}
          openAccounts={openAccounts}
          openAgents={openAgents}
          close={finishWelcome}
        />
      )}
      <ConfirmDialogs
        automatic={automatic}
        activeAnnouncements={announcements?.active}
      />
    </>
  );
}

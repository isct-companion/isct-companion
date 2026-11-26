import { useEffect, useRef, useState } from "react";
import { DataNotice } from "../components/DataNotice";
import { useWelcome, WelcomeSheet } from "../components/guide";
import {
  ConfirmDialogs,
  useConfirmationPending,
  type NoticeDialog,
} from "../components/confirm";
import { ClassGroupPrompt } from "../pages/courses/ClassGroupPicker";
import { notify } from "../components/notifications/notify";
import { useUndoAvailable } from "../components/notifications/notifications";
import { useSurfaceBlocked } from "../components/ui/modal-state";
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
  const blocked = useSurfaceBlocked();
  const undo = useUndoAvailable();
  const confirming = useConfirmationPending();
  const [group, setGroup] = useState<ClassGroupChoice>();
  const [showGroup, setShowGroup] = useState(false);
  const askedSemesters = useRef(new Set<string>());
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
        if (cancelled) return;
        const next = choices.find(
          (choice) =>
            choice.needsChoice && !askedSemesters.current.has(choice.semester),
        );
        if (next) setGroup(next);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [readOnly, kyomu, sources.length]);

  const readyForGroup =
    Boolean(group) &&
    !showGroup &&
    !needsDataNotice &&
    !needsWelcome &&
    !blocked &&
    !undo &&
    !confirming;
  useEffect(() => {
    if (!readyForGroup || !group) return;
    askedSemesters.current.add(group.semester);
    setShowGroup(true);
  }, [readyForGroup, group]);

  const automatic: NoticeDialog[] =
    needsDataNotice || needsWelcome || group
      ? []
      : [
          ...(offer && settings.appUpdateChecks !== false ? [offer] : []),
          ...(readingAnnouncements
            ? []
            : (announcements?.pending ?? []).map(announcementDialog)),
        ];

  const closeGroup = () => {
    setGroup(undefined);
    setShowGroup(false);
  };
  return (
    <>
      {needsDataNotice && (
        <DataNotice
          acknowledge={() =>
            update((draft) => {
              draft.settings.dataNoticeAcknowledgedAt =
                new Date().toISOString();
              return draft;
            })
          }
        />
      )}
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
      {group && showGroup && (
        <ClassGroupPrompt
          choice={group}
          later={closeGroup}
          choose={(value) => {
            closeGroup();
            void workspaceActions
              .setClassGroup(group.semester, value)
              .catch(notify.error);
          }}
        />
      )}
      <ConfirmDialogs
        automatic={automatic}
        activeAnnouncements={announcements?.active}
      />
    </>
  );
}

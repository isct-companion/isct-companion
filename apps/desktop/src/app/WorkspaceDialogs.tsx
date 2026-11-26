import { useEffect, useRef, useState } from "react";
import { DataNotice } from "../components/DataNotice";
import { useWelcome, WelcomeSheet } from "../components/guide";
import {
  ConfirmDialogs,
  MessageDialog,
  type AutomaticDialog,
} from "../components/confirm";
import { ClassGroupPrompt } from "../pages/courses/ClassGroupPicker";
import { notify } from "../components/notifications/notify";
import { useDesktopUpdates } from "./useDesktopUpdates";
import { useSync } from "../store/sync";
import { workspaceActions } from "../store/workspace";
import { scheduleQuery } from "../platform/core-query";
import { openExternalUrl } from "../platform/external-url";
import { tokyoDate } from "../lib/civil-date";
import { wallNow } from "../platform/use-clock";
import type { ClassGroupChoice, Workspace } from "../lib/model";
import type { AnnouncementsView } from "../generated/persistence";
import { t } from "../i18n";

export function WorkspaceDialogs({
  settings,
  update,
  readOnly,
  connected,
  openAccounts,
  openAgents,
  announcements,
}: {
  settings: Workspace["settings"];
  update: (edit: (draft: Workspace) => Workspace) => void;
  readOnly: boolean;
  connected: boolean;
  openAccounts: () => void;
  openAgents: () => void;
  announcements?: AnnouncementsView;
}) {
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

  const candidates: AutomaticDialog[] = [];
  if (!readOnly && !settings.dataNoticeAcknowledgedAt) {
    candidates.push({
      id: "data-notice",
      render: (dismiss) => (
        <DataNotice
          acknowledge={() => {
            update((draft) => {
              draft.settings.dataNoticeAcknowledgedAt =
                new Date().toISOString();
              return draft;
            });
            dismiss();
          }}
        />
      ),
    });
  }
  if (needsWelcome) {
    candidates.push({
      id: "welcome",
      live: true,
      repeat: true,
      render: (dismiss) => (
        <WelcomeSheet
          settings={settings}
          update={update}
          connected={connected}
          openAccounts={openAccounts}
          openAgents={openAgents}
          close={() => {
            finishWelcome();
            dismiss();
          }}
        />
      ),
    });
  }
  if (group) {
    candidates.push({
      id: `physics:${group.semester}`,
      render: (dismiss) => (
        <ClassGroupPrompt
          choice={group}
          later={() => {
            setGroup(undefined);
            dismiss();
          }}
          choose={(value) => {
            setGroup(undefined);
            dismiss();
            void workspaceActions
              .setClassGroup(group.semester, value)
              .catch(notify.error);
          }}
        />
      ),
    });
  }
  if (offer) candidates.push(offer);
  for (const item of announcements?.pending ?? []) {
    const link = item.link;
    const request = {
      title: item.title,
      message: item.body,
      choices: [
        ...(link ? [{ id: "details", label: link.label }] : []),
        { id: "acknowledge", label: t("announcements.acknowledge") },
      ],
    };
    candidates.push({
      id: `announcement:${item.id}`,
      announcementId: item.id,
      render: (dismiss) => (
        <MessageDialog
          request={request}
          finish={(choice) => {
            dismiss();
            if (!choice) return;
            void workspaceActions
              .mutate({ kind: "answerAnnouncement", id: item.id })
              .catch(notify.error);
            if (choice === "details" && link)
              void openExternalUrl(link.url).catch(notify.error);
          }}
        />
      ),
    });
  }
  return (
    <ConfirmDialogs
      automatic={candidates}
      activeAnnouncements={
        announcements && new Set(announcements.active.map((item) => item.id))
      }
    />
  );
}

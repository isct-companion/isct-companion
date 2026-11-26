import { useEffect, useState, type ReactNode } from "react";
import { create } from "zustand";
import { Button, Group, Modal, Stack, Text } from "./ui";
import { t } from "../i18n";
import { useTranslation } from "react-i18next";
import { useSurfaceBlocked } from "./ui/modal-state";
import { useUndoAvailable } from "./notifications/notifications";

/** One button of a dialog; the last one is the primary action unless styled. */
export type Choice<T extends string> = {
  id: T;
  label: string;
  danger?: boolean;
  variant?: "filled" | "default" | "subtle" | "light";
};
type Confirmation = {
  message: string;
  title?: string;
  choices: Choice<string>[];
  resolve: (choice: string | null) => void;
};
export type NoticeDialog = {
  id: string;
  message: ReactNode;
  title?: string;
  choices: Choice<string>[];
  announcementId?: string;
  closeOnClickOutside?: boolean;
  onChoice: (choice: string | null) => void;
};
const useConfirmations = create<{ queue: Confirmation[] }>(() => ({
  queue: [],
}));

/** In-app Radix Dialog: only the confirm button resolves true. Never use
 * window.confirm: the native shell replaces it with an asynchronous API. */
export function confirmAction(
  message: string,
  confirmLabel: string,
  { title, danger = true }: { title?: string; danger?: boolean } = {},
) {
  return chooseAction(
    message,
    [
      { id: "cancel", label: t("common.cancel") },
      { id: "confirm", label: confirmLabel, danger },
    ],
    { title },
  ).then((choice) => choice === "confirm");
}
/** As `confirmAction`, with buttons of its own: resolves the chosen id, or
 * null when the dialog is closed without one. */
export function chooseAction<T extends string>(
  message: string,
  choices: Choice<T>[],
  { title }: { title?: string } = {},
) {
  return new Promise<T | null>((resolve) =>
    useConfirmations.setState(({ queue }) => ({
      queue: [
        ...queue,
        {
          message,
          title,
          choices,
          resolve: resolve as (choice: string | null) => void,
        },
      ],
    })),
  );
}

/** User confirmations come first. Automatic offers (data notice, physics group,
 * updates, announcements) are snapshotted on open so later arrivals never
 * replace them, and an open announcement closes if Core withdraws or expires it. */
export function ConfirmDialogs({
  automatic = [],
  activeAnnouncements,
}: {
  automatic?: readonly NoticeDialog[];
  activeAnnouncements?: readonly { id: string }[];
}) {
  useTranslation();
  const request = useConfirmations((state) => state.queue[0]);
  const [open, setOpen] = useState<NoticeDialog>();
  const [offered, setOffered] = useState<ReadonlySet<string>>(() => new Set());
  const blocked = useSurfaceBlocked();
  const undo = useUndoAvailable();
  useEffect(
    () => () => {
      const queue = useConfirmations.getState().queue;
      useConfirmations.setState({ queue: [] });
      queue.forEach((item) => item.resolve(null));
    },
    [],
  );
  const next = automatic.find((item) => !offered.has(item.id));
  useEffect(() => {
    if (
      open?.announcementId &&
      activeAnnouncements &&
      !activeAnnouncements.some((item) => item.id === open.announcementId)
    ) {
      setOpen(undefined);
    } else if (!open && !request && !blocked && !undo && next) {
      setOpen(next);
      setOffered((old) => new Set(old).add(next.id));
    }
  }, [open, request, blocked, undo, next, activeAnnouncements]);

  const shown = open ?? request;
  if (!shown) return null;
  const finish = (choice: string | null) => {
    if (open) {
      setOpen(undefined);
      open.onChoice(choice);
    } else if (request) {
      useConfirmations.setState(({ queue }) => ({
        queue: queue.filter((item) => item !== request),
      }));
      request.resolve(choice);
    }
  };
  const last = shown.choices.length - 1;
  return (
    <Modal
      opened
      onClose={() => finish(null)}
      closeOnClickOutside={open?.closeOnClickOutside}
      title={shown.title}
      withCloseButton={false}
      centered
    >
      <Stack>
        {typeof shown.message === "string" ? (
          <Text size="sm" style={{ whiteSpace: "pre-wrap" }}>
            {shown.message}
          </Text>
        ) : (
          shown.message
        )}
        <Group justify="flex-end">
          {shown.choices.map((choice, index) => (
            <Button
              key={choice.id}
              variant={
                choice.variant ?? (index === last ? undefined : "default")
              }
              color={choice.danger ? "red" : undefined}
              onClick={() => finish(choice.id)}
            >
              {choice.label}
            </Button>
          ))}
        </Group>
      </Stack>
    </Modal>
  );
}

import { useEffect, useState, type ReactNode } from "react";
import { create } from "zustand";
import { Button, Group, Modal, Stack, Text } from "./ui";
import { t } from "../i18n";
import { useTranslation } from "react-i18next";
import { useAppModalOpen, useNativeModalOpen } from "./ui/modal-state";
import { useWindowActive } from "../platform/use-window-active";
import { useUndoAvailable } from "./notifications/notifications";

/** One button of a dialog; the last one is the primary action. */
export type Choice<T extends string> = {
  id: T;
  label: string;
  danger?: boolean;
};
type Confirmation = {
  message: string;
  title?: string;
  choices: Choice<string>[];
  resolve: (choice: string | null) => void;
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
export type AutomaticDialog = {
  id: string;
  render: (dismiss: () => void) => ReactNode;
  /** Forms keep reading their controls while the host owns presentation. */
  live?: boolean;
  repeat?: boolean;
  announcementId?: string;
};

/** The shell supplies eligibility in presentation order. Once open, a message
 * and its answers belong to that presentation, even if the candidates change. */
export function ConfirmDialogs({
  automatic = [],
  activeAnnouncements,
  blocked = false,
}: {
  automatic?: readonly AutomaticDialog[];
  activeAnnouncements?: ReadonlySet<string>;
  blocked?: boolean;
}) {
  useTranslation();
  const request = useConfirmations((state) => state.queue[0]);
  const [open, setOpen] = useState<AutomaticDialog>();
  const [offered, setOffered] = useState<ReadonlySet<string>>(() => new Set());
  const modal = useAppModalOpen();
  const nativeModal = useNativeModalOpen();
  const foreground = useWindowActive();
  const undo = useUndoAvailable();
  useEffect(
    () => () => {
      const queue = useConfirmations.getState().queue;
      useConfirmations.setState({ queue: [] });
      queue.forEach((item) => item.resolve(null));
    },
    [],
  );
  const next = automatic.find((item) => item.repeat || !offered.has(item.id));
  useEffect(() => {
    if (
      open?.announcementId &&
      activeAnnouncements &&
      !activeAnnouncements.has(open.announcementId)
    ) {
      setOpen(undefined);
    } else if (
      !open &&
      !request &&
      foreground &&
      !blocked &&
      !undo &&
      !modal &&
      !nativeModal &&
      next
    ) {
      setOpen(next);
      if (!next.repeat) setOffered((old) => new Set(old).add(next.id));
    }
  }, [
    open,
    request,
    foreground,
    blocked,
    undo,
    modal,
    nativeModal,
    next,
    activeAnnouncements,
  ]);

  if (open) {
    const current = open.live
      ? (automatic.find((item) => item.id === open.id) ?? open)
      : open;
    return current.render(() => setOpen(undefined));
  }
  if (!request) return null;
  return (
    <MessageDialog
      request={request}
      finish={(choice) => {
        useConfirmations.setState(({ queue }) => ({
          queue: queue.filter((item) => item !== request),
        }));
        request.resolve(choice);
      }}
    />
  );
}

export function MessageDialog<T extends string>({
  request,
  finish,
}: {
  request: { message: string; title?: string; choices: Choice<T>[] };
  finish: (choice: T | null) => void;
}) {
  const last = request.choices.length - 1;
  return (
    <Modal
      opened
      onClose={() => finish(null)}
      title={request.title}
      withCloseButton={false}
      centered
    >
      <Stack>
        <Text size="sm" style={{ whiteSpace: "pre-wrap" }}>
          {request.message}
        </Text>
        <Group justify="flex-end">
          {request.choices.map((choice, index) => (
            <Button
              key={choice.id}
              variant={index === last ? undefined : "default"}
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

import { useEffect } from "react";
import { create } from "zustand";
import { Button, Group, Modal, Stack, Text } from "./ui";
import { t } from "../i18n";
import { useTranslation } from "react-i18next";

type Confirmation = {
  message: string;
  confirmLabel: string;
  title?: string;
  danger: boolean;
  resolve: (confirmed: boolean) => void;
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
  return new Promise<boolean>((resolve) =>
    useConfirmations.setState(({ queue }) => ({
      queue: [...queue, { message, confirmLabel, title, danger, resolve }],
    })),
  );
}
/** True while a confirmation is open; a native view over the page steps
 * aside, since no HTML overlay can draw above it. */
export const useConfirmationPending = () =>
  useConfirmations((state) => state.queue.length > 0);
export function ConfirmDialogs() {
  useTranslation();
  const request = useConfirmations((state) => state.queue[0]);
  useEffect(
    () => () => {
      const queue = useConfirmations.getState().queue;
      useConfirmations.setState({ queue: [] });
      queue.forEach((item) => item.resolve(false));
    },
    [],
  );
  if (!request) return null;
  const finish = (confirmed: boolean) => {
    useConfirmations.setState(({ queue }) => ({
      queue: queue.filter((item) => item !== request),
    }));
    request.resolve(confirmed);
  };
  return (
    <Modal
      opened
      onClose={() => finish(false)}
      title={request.title}
      withCloseButton={false}
      centered
    >
      <Stack>
        <Text size="sm">{request.message}</Text>
        <Group justify="flex-end">
          <Button variant="default" onClick={() => finish(false)}>
            {t("common.cancel")}
          </Button>
          <Button
            color={request.danger ? "red" : undefined}
            onClick={() => finish(true)}
          >
            {request.confirmLabel}
          </Button>
        </Group>
      </Stack>
    </Modal>
  );
}

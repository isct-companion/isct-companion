import { useEffect } from "react";
import { create } from "zustand";
import { Button, Group, Modal, Stack, Text } from "./ui";
import { t } from "../i18n";
import { useTranslation } from "react-i18next";

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
      queue.forEach((item) => item.resolve(null));
    },
    [],
  );
  if (!request) return null;
  const finish = (choice: string | null) => {
    useConfirmations.setState(({ queue }) => ({
      queue: queue.filter((item) => item !== request),
    }));
    request.resolve(choice);
  };
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
        <Text size="sm">{request.message}</Text>
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

import { nativeOs } from "../platform/desktop-bridge";
import { Button, Group, Modal, Popover, Stack, Text } from "./ui";
import { useLocalStorage } from "usehooks-ts";
import { t, type MessageKey } from "../i18n";
import type { Workspace } from "../lib/model";
import { Choice } from "./forms";
import "./guide.css";

/**
 * First-run guidance: one welcome sheet, then one-time tips anchored to the
 * control they explain. Radix Popover and Dialog are enough for a single
 * anchored hint; a step-by-step tour library would add a mode the app does
 * not want. What has been seen is a per-device convenience, so it stays in
 * this window's storage and never syncs (the phone has its own tips).
 */
export const GUIDE_KEY = "isct-companion-guide";

/** Every tip, so tests and "Show tips again" can name them all. */
export const TIPS = ["lesson-mark", "timetable-marks", "feedback"] as const;
export type TipId = (typeof TIPS)[number];

type GuideState = { welcome?: boolean; tips?: string[] };

function useGuide() {
  return useLocalStorage<GuideState>(GUIDE_KEY, {});
}

/** Clears what this device has seen; the welcome sheet and tips come back. */
export function useResetGuide() {
  const [, setGuide] = useGuide();
  return () => setGuide({});
}

/**
 * Shows `text` once, pointing at `children`, after the welcome sheet is
 * done. `children` is the one control the tip explains (a lesson tick, a
 * class block), never the region around it: the tip only fits beside
 * something small, and pointing at the thing is what makes it read. Render
 * the tip only while that control is on screen. The control only anchors
 * the tip; it keeps its own click. Inside another `asChild` (a context menu
 * trigger), what that one hands down passes through to the control.
 */
export function GuideTip({
  id,
  text,
  position = "bottom",
  children,
  ...forwarded
}: Omit<React.ComponentProps<typeof Popover.Anchor>, "children"> & {
  id: TipId;
  text: MessageKey;
  position?: "top" | "bottom" | "left" | "right";
  children: React.ReactElement;
}) {
  const [guide, setGuide] = useGuide();
  const opened = guide.welcome === true && !guide.tips?.includes(id);
  const dismiss = () =>
    setGuide((current) => ({
      ...current,
      tips: [...new Set([...(current.tips ?? []), id])],
    }));
  return (
    <Popover
      opened={opened}
      position={position}
      withArrow
      arrowSize={10}
      shadow="md"
      width={260}
      withinPortal
      trapFocus={false}
      closeOnClickOutside={false}
      onDismiss={dismiss}
    >
      <Popover.Anchor {...forwarded}>{children}</Popover.Anchor>
      <Popover.Dropdown className="guideTip" role="note">
        <Text size="sm">{t(text)}</Text>
        <Group justify="flex-end" mt="space-4">
          <Button size="compact-xs" variant="light" onClick={dismiss}>
            {t("guide.gotIt")}
          </Button>
        </Group>
      </Popover.Dropdown>
    </Popover>
  );
}

/**
 * The first window: the two language choices people did not know existed,
 * then one way on to the school accounts and one to AI agents.
 */
export function useWelcome() {
  const [guide, setGuide] = useGuide();
  return [
    guide.welcome !== true,
    () => setGuide((current) => ({ ...current, welcome: true })),
  ] as const;
}

export function WelcomeSheet({
  settings,
  update,
  connected,
  close,
  openAccounts,
  openAgents,
}: {
  settings: Workspace["settings"];
  close: () => void;
  update: (f: (d: Workspace) => Workspace) => void;
  connected: boolean;
  openAccounts: () => void;
  openAgents: () => void;
}) {
  const set = <K extends "locale" | "contentLanguage">(
    key: K,
    value: Workspace["settings"][K],
  ) =>
    update((draft) => {
      draft.settings[key] = value;
      return draft;
    });
  return (
    <Modal
      opened
      onClose={close}
      title={t("common.welcomeToIsctCompanion")}
      // Top-anchored: the language rows rewrap when the language changes,
      // and a centred sheet would slide them under the pointer.
      size="md"
      classNames={{ body: "welcomeSheet" }}
    >
      <Stack gap="space-7">
        <div className="welcomeRow">
          <Text fw={600} size="sm">
            {t("common.appLanguage")}
          </Text>
          <Choice
            value={settings.locale}
            options={[
              { value: "en", label: "English" },
              { value: "ja", label: "日本語" },
              { value: "zh", label: "中文" },
            ]}
            onChange={(locale) => set("locale", locale)}
          />
        </div>
        <div className="welcomeRow">
          <div>
            <Text fw={600} size="sm">
              {t("guide.courseAndSyllabusLanguage")}
            </Text>
            <Text size="xs" c="dimmed">
              {t("guide.canDifferFromTheApp")}
            </Text>
          </div>
          <Choice
            value={settings.contentLanguage ?? "auto"}
            options={[
              { value: "auto", label: t("common.automatic") },
              { value: "en", label: "English" },
              { value: "ja", label: "日本語" },
            ]}
            onChange={(language) => set("contentLanguage", language)}
          />
        </div>
        {nativeOs() === "macos" && (
          <Text size="xs" c="dimmed">
            {t("widget.addOnMac")}
          </Text>
        )}
        <button
          type="button"
          className="welcomeLink"
          onClick={() => {
            close();
            openAgents();
          }}
        >
          <b>{t("guide.useWithClaudeCodexOr")}</b>
          <span>{t("guide.connectAnAIAssistantOver")} →</span>
        </button>
        <Group justify="flex-end" gap="space-4">
          {/* Focus starts on the main button, not the close box, so Return
              and Space never dismiss the sheet for good by accident. */}
          <Button
            variant="subtle"
            color="gray"
            autoFocus={connected}
            onClick={close}
          >
            {t(connected ? "common.done" : "common.later")}
          </Button>
          {!connected && (
            <Button
              autoFocus
              onClick={() => {
                close();
                openAccounts();
              }}
            >
              {t("common.connectSchoolAccounts")}
            </Button>
          )}
        </Group>
      </Stack>
    </Modal>
  );
}

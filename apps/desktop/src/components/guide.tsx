import { Button, Group, Modal, Popover, Stack, Text } from "@mantine/core";
import { useLocalStorage } from "@mantine/hooks";
import { t } from "../i18n";
import type { Workspace } from "../lib/model";
import { Choice } from "./forms";
import "./guide.css";

/**
 * First-run guidance: one welcome sheet, then one-time tips anchored to the
 * control they explain. Mantine's Popover and Modal are enough for a single
 * anchored hint; a step-by-step tour library would add a mode the app does
 * not want. What has been seen is a per-device convenience, so it stays in
 * this window's storage and never syncs (the phone has its own tips).
 */
export const GUIDE_KEY = "isct-companion-guide";

/** Every tip, so tests and "Show tips again" can name them all. */
export const TIPS = ["lesson-mark", "timetable-marks"] as const;
export type TipId = (typeof TIPS)[number];

type GuideState = { welcome?: boolean; tips?: string[] };

function useGuide() {
  return useLocalStorage<GuideState>({
    key: GUIDE_KEY,
    defaultValue: {},
    getInitialValueInEffect: false,
  });
}

/** Clears what this device has seen; the welcome sheet and tips come back. */
export function useResetGuide() {
  const [, setGuide] = useGuide();
  return () => setGuide({});
}

/**
 * Shows `text` once beside `children`, after the welcome sheet is done.
 * `when` holds a tip back until the thing it explains is on screen.
 */
export function GuideTip({
  id,
  text,
  when = true,
  position = "bottom",
  children,
}: {
  id: TipId;
  text: string;
  when?: boolean;
  position?: "top" | "bottom" | "left" | "right";
  children: React.ReactNode;
}) {
  const [guide, setGuide] = useGuide();
  const opened = when && guide.welcome === true && !guide.tips?.includes(id);
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
      <Popover.Target>{children}</Popover.Target>
      <Popover.Dropdown className="guideTip" role="note">
        <Text size="sm">{t(text)}</Text>
        <Group justify="flex-end" mt={8}>
          <Button size="compact-xs" variant="light" onClick={dismiss}>
            {t("Got it")}
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
export function WelcomeSheet({
  settings,
  update,
  connected,
  held = false,
  openAccounts,
  openAgents,
}: {
  settings: Workspace["settings"];
  /** Another first-run dialog (the data notice) comes first. */
  held?: boolean;
  update: (f: (d: Workspace) => Workspace) => void;
  connected: boolean;
  openAccounts: () => void;
  openAgents: () => void;
}) {
  const [guide, setGuide] = useGuide();
  const close = () => setGuide((current) => ({ ...current, welcome: true }));
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
      opened={guide.welcome !== true && !held}
      onClose={close}
      title={t("Welcome to IsctCompanion")}
      centered
      size="md"
      classNames={{ body: "welcomeSheet" }}
    >
      <Stack gap="md">
        <div className="welcomeRow">
          <Text fw={600} size="sm">
            {t("App language")}
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
              {t("Course and syllabus language")}
            </Text>
            <Text size="xs" c="dimmed">
              {t("Can differ from the app language.")}
            </Text>
          </div>
          <Choice
            value={settings.contentLanguage ?? "auto"}
            options={[
              { value: "auto", label: t("Automatic") },
              { value: "en", label: "English" },
              { value: "ja", label: "日本語" },
            ]}
            onChange={(language) => set("contentLanguage", language)}
          />
        </div>
        <button
          type="button"
          className="welcomeLink"
          onClick={() => {
            close();
            openAgents();
          }}
        >
          <b>{t("Use with Claude, Codex or Cursor")}</b>
          <span>{t("Connect an AI assistant over MCP")} →</span>
        </button>
        <Group justify="flex-end" gap="xs">
          <Button variant="subtle" color="gray" onClick={close}>
            {t(connected ? "Done" : "Later")}
          </Button>
          {!connected && (
            <Button
              onClick={() => {
                close();
                openAccounts();
              }}
            >
              {t("Connect school accounts")}
            </Button>
          )}
        </Group>
      </Stack>
    </Modal>
  );
}

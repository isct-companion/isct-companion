// @vitest-environment jsdom
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  buttonByText,
  click,
  chooseOption,
  typeInto,
  flush,
  advance,
  mountApp,
  overviewFor,
  planningWeekFor,
  lectureListFor,
  until,
  workspaceFixture,
  YEAR,
  type FakeCore,
  type FakeCoreOptions,
  type MountedApp,
  readRepoFile,
  schoolProfiles,
} from "@tests/harness";
import type {
  SchoolCredentialStatus,
  SchoolProfileSession,
} from "@app/generated/persistence";
import type { CloudSyncState } from "@app/platform/desktop-bridge";
import type { StudyEnrollment, Workspace } from "@app/lib/model";
import { paths } from "@app/app/routes";
import { schoolSessions } from "@app/platform/school-session";

/**
 * The settings pages through the real shell: school accounts and the Matrix
 * editor, backup and cloud sync, the study profile, and local agent access.
 *
 * Everything runs over the scripted host installed by `mountApp`, so the real
 * bridge, query cache and workspace store drive the real pages. Each case
 * checks that its path goes through and where the host's state ends; what a
 * page draws is for a person to look at (docs/verification/manual-checks.yaml).
 * Platform seams are stubbed only where jsdom cannot supply the native
 * runtime:
 *
 * - `navigator.clipboard` is absent in jsdom, so the agent suite installs one.
 * - ZXing camera/canvas decoding is replaced by controlled results; account
 *   selection, draft replacement and explicit credential saves remain real.
 *
 * Matrix OCR is not driven here: recognition on real card photos is app-ocr's
 * (the MATRIX_PHOTO test) and the camera and photo flows are OCR-CAMPUS-CARD.
 *
 * `mountApp` must stay the first thing that imports the shell: `platform/native.ts`
 * captures its bridge at first import.
 */

// The first case in the file also pays the cold import of the whole shell, and
// the shell's boot takes seconds under jsdom; the cap only bites on failure.
const TIMEOUT = 60_000;

type Handler = NonNullable<FakeCoreOptions["handle"]>;
type Apply = NonNullable<FakeCoreOptions["apply"]>;

// ZXing needs native camera/canvas APIs unavailable in jsdom. Only the
// decoder transport is replaced; the real input, import intent and save flow run.
const qrDecode = vi.hoisted(() => vi.fn());
const qrVideo = vi.hoisted(() => vi.fn());
vi.mock("@zxing/browser", () => ({
  BrowserQRCodeReader: class {
    decodeFromImageUrl = qrDecode;
    decodeFromVideoDevice = qrVideo;
  },
}));

let app: MountedApp | undefined;

beforeEach(() => {
  // `schoolSessions` is a module singleton that outlives a single mount.
  schoolSessions.reset();
});

afterEach(async () => {
  await app?.unmount();
  app = undefined;
  vi.unstubAllGlobals();
  qrDecode.mockReset();
  qrVideo.mockReset();
});

/** A wait long enough for a cold shell boot on a loaded machine. */
const wait = <T,>(check: () => T): Promise<T> =>
  until(check, { timeout: 10_000 });

function settingsPane(mounted: MountedApp): HTMLElement {
  const pane = mounted.host.querySelector<HTMLElement>(".settingsPane");
  if (!pane) throw new Error("The settings pane is not mounted.");
  return pane;
}

function buttonIn(scope: ParentNode, label: string): HTMLButtonElement {
  const element = buttonByText(scope, label);
  if (!element) throw new Error(`Missing the ${label} button.`);
  return element;
}

async function clickButton(scope: ParentNode, label: string): Promise<void> {
  await click(buttonIn(scope, label));
}

function openDialog(): HTMLElement {
  const dialogs = document.querySelectorAll<HTMLElement>('[role="dialog"]');
  if (!dialogs.length) throw new Error("No confirm dialog is open.");
  return dialogs[dialogs.length - 1];
}

async function answerDialog(label: string): Promise<void> {
  const dialog = await wait(() => openDialog());
  await clickButton(dialog, label);
}

/** Sends a native event to the shell under test. */
function emitNative(core: FakeCore, event: string, payload: unknown): void {
  core.emit(event, payload);
}

function mountSettings(
  route: string,
  options: {
    workspace?: Workspace;
    capabilities?: Record<string, unknown>;
    handle?: Handler;
    apply?: Apply;
    queries?: FakeCoreOptions["queries"];
    tools?: FakeCoreOptions["tools"];
    finder?: FakeCoreOptions["finder"];
  } = {},
): Promise<MountedApp> {
  return mountApp(
    {
      workspace: options.workspace ?? workspaceFixture(),
      tools: options.tools,
      finder: options.finder,
      capabilities: options.capabilities ?? {},
      queries: {
        workspaceOverview: (_query, core) => overviewFor(core.workspace),
        planningWeek: (_query, core) => planningWeekFor(core.workspace),
        lectureList: (_query, core) => lectureListFor(core.workspace),
        academicYears: { currentYear: YEAR },
        ...options.queries,
      },
      handle: options.handle,
      apply: options.apply,
    },
    { route },
  );
}

/* ------------------------------------------------------------------ accounts */

type Portal = SchoolCredentialStatus["science"];

const notConfigured = (): Portal => ({
  availability: "not-configured",
  saved: false,
  username: "",
  hasOtp: false,
  hasMatrix: false,
});

const saved = (overrides: Partial<Portal> = {}): Portal => ({
  availability: "saved-locked",
  saved: true,
  username: "",
  hasOtp: false,
  hasMatrix: false,
  ...overrides,
});

const noSession = {
  revision: 0,
  profiles: schoolProfiles("not-configured", "not-configured"),
};

/** Core's answer to a Connect that signed in and saved the credentials. */
const connected = {
  kind: "connected",
  presentation: {
    stateKey: "schoolSession.connect.connected",
    reasonKey: null,
    recovery: [],
  },
};

function account(
  mounted: MountedApp,
  profile: "science" | "tokyo-tech",
): HTMLElement {
  const card = mounted.host.querySelector<HTMLElement>(
    `section[data-setting="${profile}"]`,
  );
  if (!card) throw new Error(`The ${profile} account card is not mounted.`);
  return card;
}

async function mountAccounts(
  options: {
    capabilities?: Record<string, unknown>;
    handle?: Handler;
  } = {},
): Promise<MountedApp> {
  const mounted = await mountSettings(paths.settings("accounts"), options);
  app = mounted;
  await wait(() => account(mounted, "science"));
  return mounted;
}

/** Picks one of the OTP setup paths, a radio card whose name starts with its
 * title and goes on with its hint. */
async function chooseOtpPath(card: HTMLElement, title: string): Promise<void> {
  const path = [
    ...card.querySelectorAll<HTMLButtonElement>('[role="radio"]'),
  ].find((radio) => radio.textContent?.startsWith(title));
  if (!path) throw new Error(`Missing the ${title} OTP path.`);
  await click(path);
}

const PNG = () => new File(["test"], "card.png", { type: "image/png" });

async function chooseFile(input: HTMLInputElement, file: File): Promise<void> {
  Object.defineProperty(input, "files", { configurable: true, value: [file] });
  await act(async () =>
    input.dispatchEvent(new Event("change", { bubbles: true })),
  );
}

async function scanFile(card: HTMLElement, file: File): Promise<void> {
  const input = card.querySelector<HTMLInputElement>('input[type="file"]');
  if (!input) throw new Error("The scan input is not mounted.");
  await chooseFile(input, file);
}

describe("School accounts", () => {
  it(
    "imports a selected QR account into the draft and saves only on Connect",
    async () => {
      const setup =
        "otpauth://totp/School?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
      qrDecode.mockResolvedValue({ getText: () => "google-export-fixture" });
      const revoke = vi.fn();
      vi.stubGlobal(
        "URL",
        class extends URL {
          static createObjectURL() {
            return "blob:qr-test";
          }
          static revokeObjectURL = revoke;
        },
      );
      const mounted = await mountAccounts({
        handle: (command) => {
          if (command === "school_credential_status")
            return {
              science: saved({ hasOtp: true }),
              tokyoTech: notConfigured(),
            };
          if (command === "school_session_snapshot") return noSession;
          if (command === "copy_school_otp") return true;
          if (command === "import_school_otp")
            return [
              {
                label: "Other account",
                issuer: "Other",
                setup: "other-secret",
              },
              { label: "School account", issuer: "School", setup },
            ];
          if (command === "connect_school") return connected;
          return undefined;
        },
      });
      const science = account(mounted, "science");
      await click(
        science.querySelector<HTMLButtonElement>(
          '[aria-label="Copy OTP setup"]',
        )!,
      );
      await wait(() =>
        expect(mounted.core.commands("copy_school_otp")[0]?.args).toEqual({
          setup: undefined,
        }),
      );
      await clickButton(science, "Change");
      await chooseOtpPath(science, "Import existing OTP");
      await scanFile(science, PNG());
      await click(
        await wait(() => buttonIn(document.body, "School · School account")),
      );
      expect(revoke).toHaveBeenCalledWith("blob:qr-test");
      await click(
        science.querySelector<HTMLButtonElement>(
          '[aria-label="Copy OTP setup"]',
        )!,
      );
      await wait(() =>
        expect(mounted.core.commands("copy_school_otp").at(-1)?.args).toEqual({
          setup,
        }),
      );
      expect(mounted.core.commands("connect_school")).toHaveLength(0);
      await clickButton(science, "Connect");
      await wait(() =>
        expect(mounted.core.commands("connect_school")[0].args).toMatchObject({
          credentials: { otpSecret: setup },
        }),
      );
    },
    TIMEOUT,
  );

  it(
    "stops a QR camera that finishes opening after the dialog is closed",
    async () => {
      let resolveCamera: (controls: { stop: () => void }) => void = () => {};
      qrVideo.mockImplementation(
        () =>
          new Promise((resolve) => {
            resolveCamera = resolve;
          }),
      );
      const mounted = await mountAccounts();
      await chooseOtpPath(account(mounted, "science"), "Import existing OTP");
      await clickButton(account(mounted, "science"), "Scan QR");
      await wait(() => expect(qrVideo).toHaveBeenCalled());
      const close = document.querySelector<HTMLButtonElement>(
        '.uiDialogContent button[aria-label="Close"]',
      )!;
      await click(close);
      const stop = vi.fn();
      await act(async () => resolveCamera({ stop }));
      await wait(() => expect(stop).toHaveBeenCalled());
      expect(mounted.core.commands("connect_school")).toHaveLength(0);
    },
    TIMEOUT,
  );

  it(
    "connects in one call, keeps a draft the school refused or the vault did not save and clears it once connected",
    async () => {
      const status: SchoolCredentialStatus = {
        science: saved({ username: "saved-id", hasOtp: true }),
        tokyoTech: notConfigured(),
      };
      const answers = [
        {
          kind: "refused",
          reason: "invalid-otp",
          refusalUnsaved: false,
          presentation: {
            stateKey: "schoolSession.state.refused",
            reasonKey: "failure.theOneTimePasswordWas",
            recovery: [],
          },
        },
        {
          kind: "not-saved",
          presentation: {
            stateKey: "schoolSession.connect.saveFailed",
            reasonKey: null,
            recovery: [],
          },
        },
        connected,
      ];
      const mounted = await mountAccounts({
        handle: (command) => {
          if (command === "school_credential_status") return status;
          if (command === "school_session_snapshot") return noSession;
          if (command === "connect_school") return answers.shift();
          return undefined;
        },
      });

      const science = account(mounted, "science");
      const password = science.querySelector<HTMLInputElement>(
        'input[autocomplete="current-password"]',
      )!;
      const connectAndSettle = async (calls: number) => {
        await clickButton(science, "Connect");
        await wait(() =>
          expect(mounted.core.commands("connect_school")).toHaveLength(calls),
        );
        await wait(() =>
          expect(buttonIn(science, "Connect").disabled).toBe(false),
        );
      };
      await typeInto(password, "new-secret");

      // Refused by the school: nothing was saved, so the draft stays.
      await connectAndSettle(1);
      expect(mounted.core.commands("connect_school")[0].args).toMatchObject({
        profile: "science",
        credentials: { username: "saved-id", password: "new-secret" },
      });
      expect(password.value).toBe("new-secret");

      // Accepted by the school but not saved by the vault: the draft stays.
      await connectAndSettle(2);
      expect(password.value).toBe("new-secret");

      // Only a connection saves the draft, and the form empties.
      await connectAndSettle(3);
      expect(mounted.core.commands("connect_school")[2].args).toMatchObject({
        credentials: { password: "new-secret" },
      });
      await wait(() => expect(password.value).toBe(""));
    },
    TIMEOUT,
  );

  it(
    "retries a refused account in one call and sends one without a school OTP to the portal",
    async () => {
      const status: SchoolCredentialStatus = {
        science: saved({ username: "saved-id", hasOtp: true }),
        tokyoTech: notConfigured(),
      };
      const snapshot = (
        revision: number,
        science: Partial<SchoolProfileSession>,
      ) => {
        const profiles = schoolProfiles("normal", "not-configured");
        return {
          revision,
          profiles: {
            ...profiles,
            science: { ...profiles.science, ...science },
          },
        };
      };
      const mounted = await mountAccounts({
        handle: (command) => {
          if (command === "school_credential_status") return status;
          if (command === "school_session_snapshot") return noSession;
          if (command === "retry_school")
            return snapshot(4, { state: "normal" });
          return undefined;
        },
      });
      const science = account(mounted, "science");
      await wait(() =>
        expect(mounted.core.listenerCount("school-session")).toBe(1),
      );

      // Core offers Retry for a refusal: one call that keeps the saved
      // credentials instead of sending them again.
      await act(async () =>
        emitNative(
          mounted.core,
          "school-session",
          snapshot(2, {
            state: "refused",
            presentation: {
              stateKey: "schoolSession.state.refused",
              reasonKey: "failure.theIDOrPasswordWas",
              recovery: ["retry"],
            },
            problem: { kind: "invalid-credentials", message: "HTTP 401" },
          }),
        ),
      );
      await click(await wait(() => buttonIn(science, "Retry")));
      await wait(() =>
        expect(mounted.core.commands("retry_school")).toHaveLength(1),
      );
      expect(mounted.core.commands("retry_school")[0].args).toEqual({
        profile: "science",
      });
      expect(mounted.core.commands("connect_school")).toHaveLength(0);

      // The emailed-code page means the account has no school OTP: the way
      // on is the portal as it is, unsigned, where the person sets one up.
      await act(async () =>
        emitNative(
          mounted.core,
          "school-session",
          snapshot(5, {
            state: "needs-web",
            presentation: {
              stateKey: "schoolSession.state.otpNotSetUp",
              reasonKey: "schoolSession.problem.emailOtp",
              recovery: ["set-up-otp-on-portal"],
            },
            problem: { kind: "email-otp", message: "email" },
          }),
        ),
      );
      await click(
        await wait(() => buttonIn(science, "Open Science Tokyo Portal")),
      );
      expect(sessionStorage.getItem("isct-companion-active-school-site")).toBe(
        "science-portal",
      );
      expect(
        JSON.parse(sessionStorage.getItem("isct-companion-school-entry")!),
      ).toEqual({ kind: "unsigned" });
    },
    TIMEOUT,
  );

  it(
    "forgets the saved portal only after confirming",
    async () => {
      const status: SchoolCredentialStatus = {
        science: saved({ username: "saved-id", hasOtp: true }),
        tokyoTech: notConfigured(),
      };
      const mounted = await mountAccounts({
        handle: (command) => {
          if (command === "school_credential_status") return status;
          if (command === "school_session_snapshot") return noSession;
          if (command === "delete_school_credentials") {
            status.science = notConfigured();
            return true;
          }
          return undefined;
        },
      });

      await clickButton(account(mounted, "science"), "Remove");
      const confirm = await wait(() => openDialog());
      // Asking is not doing.
      expect(mounted.core.commands("delete_school_credentials")).toHaveLength(
        0,
      );

      await clickButton(confirm, "Remove");
      await wait(() =>
        expect(mounted.core.commands("delete_school_credentials")).toHaveLength(
          1,
        ),
      );
      expect(
        mounted.core.commands("delete_school_credentials")[0].args,
      ).toEqual({
        profile: "science",
      });
    },
    TIMEOUT,
  );
});

/* -------------------------------------------------------------- matrix input */

const matrixCard = (mounted: MountedApp) => account(mounted, "tokyo-tech");

function matrixInputs(card: HTMLElement): HTMLInputElement[] {
  return [...card.querySelectorAll<HTMLInputElement>(".matrixGrid input")];
}

async function pasteInto(input: HTMLInputElement, text: string): Promise<void> {
  const event = new Event("paste", { bubbles: true, cancelable: true });
  Object.defineProperty(event, "clipboardData", {
    value: { getData: () => text },
  });
  await act(async () => input.dispatchEvent(event));
}

/** A saved Tokyo Tech portal whose Matrix stays hidden until a replacement. */
const matrixStatus = (
  overrides: Partial<Portal> = {},
): SchoolCredentialStatus => ({
  science: saved({ username: "saved-id", hasOtp: true }),
  tokyoTech: saved({ username: "id", hasMatrix: true, ...overrides }),
});

describe("Matrix input", () => {
  it(
    "refuses an incomplete Matrix, keeps a pasted one through a failed save and drops it from the page once saved",
    async () => {
      // Core owns the reading (crates/app-core school_auth tests); the scripted
      // host answers with distinct letters, so a grid filled from the focused
      // cell instead of A1 would show.
      const cells = Array.from({ length: 70 }, (_, index) =>
        String.fromCharCode(65 + (index % 26)),
      ).join("");
      const status: SchoolCredentialStatus = {
        science: notConfigured(),
        tokyoTech: notConfigured(),
      };
      let vaultUp = false;
      const mounted = await mountAccounts({
        handle: (command) => {
          if (command === "school_credential_status") return status;
          if (command === "school_session_snapshot") return noSession;
          if (command === "parse_school_matrix")
            return { cells, order: "rows" };
          if (command === "connect_school") {
            if (!vaultUp) throw new Error("Vault unavailable");
            status.tokyoTech = saved({ username: "id", hasMatrix: true });
            return connected;
          }
          return undefined;
        },
      });
      const card = matrixCard(mounted);
      const inputs = await wait(() => {
        const all = matrixInputs(card);
        if (all.length !== 70) throw new Error("The Matrix grid is not ready.");
        return all;
      });
      const grid = () =>
        matrixInputs(card)
          .map((input) => input.value)
          .join("");

      // One letter is not a Matrix: Connect sends nothing and keeps it.
      await typeInto(inputs[0], "L");
      await clickButton(card, "Connect");
      await flush();
      expect(mounted.core.commands("connect_school")).toHaveLength(0);
      expect(grid()).toBe("L");

      // A whole Matrix pasted into a middle cell fills the grid from A1, as
      // Core read it from the pasted text.
      const rows = "abcdefghij ".repeat(7);
      await pasteInto(inputs[5], rows);
      await wait(() => expect(grid()).toBe(cells));
      expect(mounted.core.commands("parse_school_matrix")[0]?.args).toEqual({
        text: rows,
      });

      // A save that could not run keeps the Matrix for another try.
      await clickButton(card, "Connect");
      await wait(() =>
        expect(mounted.core.commands("connect_school")).toHaveLength(1),
      );
      await wait(() => expect(buttonIn(card, "Connect").disabled).toBe(false));
      expect(grid()).toBe(cells);

      vaultUp = true;
      await clickButton(card, "Connect");
      await wait(() =>
        expect(mounted.core.commands("connect_school")).toHaveLength(2),
      );
      for (const call of mounted.core.commands("connect_school"))
        expect(call.args).toMatchObject({
          profile: "tokyo-tech",
          credentials: { matrix: cells },
        });
      // Saved, the Matrix lives only in the vault: no letter stays in the page.
      await wait(() => expect(grid()).toBe(""));
    },
    TIMEOUT,
  );

  it(
    "asks before copying the saved plaintext Matrix",
    async () => {
      const status = matrixStatus();
      const mounted = await mountAccounts({
        handle: (command) => {
          if (command === "school_credential_status") return status;
          if (command === "school_session_snapshot") return noSession;
          if (command === "copy_school_matrix") return true;
          return undefined;
        },
      });
      const card = matrixCard(mounted);
      await wait(() => buttonIn(card, "Copy"));

      await clickButton(card, "Copy");
      const confirm = await wait(() => openDialog());
      await clickButton(confirm, "Cancel");
      await wait(() =>
        expect(document.querySelector('[role="dialog"]')).toBeNull(),
      );
      expect(mounted.core.commands("copy_school_matrix")).toHaveLength(0);

      // Escape must settle the pending confirmation as false, just like Cancel.
      const copyButton = buttonIn(card, "Copy");
      copyButton.focus();
      await clickButton(card, "Copy");
      const keyboardDismissal = await wait(() => openDialog());
      await act(async () =>
        keyboardDismissal.dispatchEvent(
          new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
        ),
      );
      await wait(() =>
        expect(document.querySelector('[role="dialog"]')).toBeNull(),
      );
      expect(mounted.core.commands("copy_school_matrix")).toHaveLength(0);
      await wait(() => expect(document.activeElement).toBe(copyButton));

      await clickButton(card, "Copy");
      await answerDialog("Copy");
      await wait(() =>
        expect(mounted.core.commands("copy_school_matrix")).toHaveLength(1),
      );
    },
    TIMEOUT,
  );
});

it(
  "keeps the application log bounded, pauses following while reading older entries, and exports it",
  async () => {
    const entries = Array.from({ length: 1500 }, (_, index) => ({
      id: index + 1,
      at: "2026-10-17T01:00:00Z",
      level: "INFO",
      target: "cloud-sync",
      message: `Confirmed log ${index + 1}`,
    }));
    app = await mountSettings(paths.settings("developer"), {
      handle: (command, args) => {
        if (command === "application_logs")
          return {
            entries: args.after ? [] : entries,
            cursor: 1500,
            dropped: false,
          };
        if (command === "export_application_logs") return "exported log text";
        if (command === "save_file" || command === "copy_application_logs")
          return true;
        return undefined;
      },
    });
    const toggle = await wait(() =>
      settingsPane(app!).querySelector<HTMLInputElement>(
        '[data-setting="developer-mode"] [role="switch"]',
      )!,
    );
    await click(toggle);
    await app.go(paths.settings("diagnostics"));
    const pane = await wait(() => {
      const current = settingsPane(app!);
      expect(
        current.querySelectorAll(".diagnosticLogRow").length,
      ).toBeGreaterThan(0);
      return current;
    });
    // 1500 entries are a scrolled window over the log, not 1500 rows.
    expect(pane.querySelectorAll(".diagnosticLogRow").length).toBeLessThan(40);
    const follow = pane.querySelector<HTMLInputElement>('[role="switch"]')!;
    expect(follow.getAttribute("aria-checked")).toBe("true");
    await act(async () =>
      pane
        .querySelector(".diagnosticLogs")!
        .dispatchEvent(
          new WheelEvent("wheel", { deltaY: -100, bubbles: true }),
        ),
    );
    expect(follow.getAttribute("aria-checked")).toBe("false");
    await click(follow);
    expect(follow.getAttribute("aria-checked")).toBe("true");

    await clickButton(pane, "Save to file");
    await wait(() => expect(app!.core.commands("save_file")).toHaveLength(1));
    expect(app.core.commands("save_file")[0].args.contents).toBe(
      "exported log text",
    );
    await clickButton(pane, "Copy");
    await wait(() =>
      expect(app!.core.commands("copy_application_logs")).toHaveLength(1),
    );
  },
  TIMEOUT,
);

/* -------------------------------------------------------------------- backup */

function cloudState(overrides: Partial<CloudSyncState> = {}): CloudSyncState {
  return {
    syncing: false,
    providers: [
      {
        provider: "icloud",
        enabled: true,
        lastSyncedAt: "2026-10-12T03:00:00Z",
      },
      { provider: "google-drive", enabled: false },
    ],
    versions: [
      {
        id: "v1",
        provider: "icloud",
        savedAt: "2026-10-12T02:00:00Z",
        device: "MacBook",
      },
    ],
    ...overrides,
  };
}

const driveToggle = (pane: HTMLElement) =>
  pane.querySelector<HTMLInputElement>(
    '[data-setting="cloud"] [role="switch"]',
  )!;

async function mountBackup(
  options: { handle?: Handler } = {},
): Promise<MountedApp> {
  const mounted = await mountSettings(paths.settings("backup"), {
    capabilities: { cloudSync: true },
    handle: options.handle,
  });
  app = mounted;
  // Both come from Core's sync state: its providers and its kept version.
  await wait(() => {
    const pane = settingsPane(mounted);
    if (!driveToggle(pane)) throw new Error("The providers are not ready.");
    buttonIn(pane, "Restore");
  });
  return mounted;
}

const autoToggle = (pane: HTMLElement) =>
  pane.querySelector<HTMLInputElement>(
    '[data-setting="sync-automatic"] [role="switch"]',
  )!;

describe("Backup", () => {
  it(
    "offers the encrypted credential flow only once Google Drive is on",
    async () => {
      const mounted = await mountBackup({
        handle: (command, args) => {
          if (command === "cloud_sync_state") return cloudState();
          if (command === "set_cloud_sync_provider") {
            const enabled = args.enabled === true;
            return cloudState({
              providers: [
                {
                  provider: "icloud",
                  enabled: true,
                  lastSyncedAt: "2026-10-12T03:00:00Z",
                },
                { provider: "google-drive", enabled },
              ],
            });
          }
          if (command === "cloud_credential_sync_status") return false;
          if (command === "cloud_credential_backup_status") return false;
          return undefined;
        },
      });
      const pane = settingsPane(mounted);
      const credentialSync = () =>
        pane.querySelector<HTMLElement>('[data-setting="credential-sync"]');
      // The credentials go to Drive only, so with Drive off there is no flow.
      expect(credentialSync()).toBeNull();

      await click(driveToggle(pane));
      await wait(() =>
        expect(mounted.core.commands("set_cloud_sync_provider")).toHaveLength(
          1,
        ),
      );
      expect(
        mounted.core.commands("set_cloud_sync_provider")[0].args,
      ).toMatchObject({
        provider: "google-drive",
        enabled: true,
      });

      await click(await wait(() => buttonIn(credentialSync()!, "Set up")));
      expect(
        pane.querySelectorAll('input[type="password"]').length,
      ).toBeGreaterThan(0);
    },
    TIMEOUT,
  );

  it(
    "uploads, pulls, and restores a kept version only after confirming",
    async () => {
      const mounted = await mountBackup({
        handle: (command) => {
          if (command === "cloud_sync_state") return cloudState();
          if (command === "sync_cloud_now") return cloudState();
          if (command === "restore_cloud_version") return cloudState();
          return undefined;
        },
      });
      const pane = settingsPane(mounted);

      await clickButton(pane, "Upload");
      await wait(() =>
        expect(mounted.core.commands("sync_cloud_now")).toHaveLength(1),
      );
      expect(mounted.core.commands("sync_cloud_now")[0].args).toMatchObject({
        direction: "upload",
      });

      await wait(() => expect(buttonIn(pane, "Pull").disabled).toBe(false));
      await clickButton(pane, "Pull");
      await wait(() =>
        expect(mounted.core.commands("sync_cloud_now")).toHaveLength(2),
      );
      expect(mounted.core.commands("sync_cloud_now")[1].args).toMatchObject({
        direction: "pull",
      });

      await wait(() => expect(buttonIn(pane, "Restore").disabled).toBe(false));
      await clickButton(pane, "Restore");
      const confirm = await wait(() => openDialog());
      expect(mounted.core.commands("restore_cloud_version")).toHaveLength(0);

      await clickButton(confirm, "Restore");
      await wait(() =>
        expect(mounted.core.commands("restore_cloud_version")).toHaveLength(1),
      );
      expect(mounted.core.commands("restore_cloud_version")[0].args).toEqual({
        id: "v1",
      });
    },
    TIMEOUT,
  );

  it(
    "switches Auto explicitly and holds Restore only while a transfer runs",
    async () => {
      const mounted = await mountBackup({
        handle: (command, args) => {
          if (command === "cloud_sync_state") return cloudState();
          if (command === "set_cloud_sync_automatic")
            return cloudState({ automatic: args.automatic === true });
          return undefined;
        },
      });
      const pane = settingsPane(mounted);

      await click(autoToggle(pane));
      await wait(() =>
        expect(mounted.core.commands("set_cloud_sync_automatic")).toHaveLength(
          1,
        ),
      );
      expect(mounted.core.commands("set_cloud_sync_automatic")[0].args).toEqual(
        {
          automatic: true,
        },
      );

      // A transfer Core reports as running holds Restore.
      await act(async () =>
        emitNative(
          mounted.core,
          "cloud-sync-state",
          cloudState({ automatic: true, syncing: true }),
        ),
      );
      await wait(() => expect(buttonIn(pane, "Restore").disabled).toBe(true));

      // A restore is an ordinary newest edit: a provider still retrying an
      // upload does not block it.
      await act(async () =>
        emitNative(
          mounted.core,
          "cloud-sync-state",
          cloudState({
            automatic: true,
            localVersion: "v1-new",
            tasks: [
              {
                id: "task-1",
                direction: "upload",
                version: "v1-new",
                createdAt: "2026-10-12T03:00:00Z",
                deliveries: [
                  {
                    provider: "icloud",
                    status: "pending",
                    attempts: 2,
                    retryAt: 1,
                  },
                  {
                    provider: "google-drive",
                    status: "completed",
                    attempts: 1,
                    retryAt: 0,
                  },
                ],
              },
            ],
          }),
        ),
      );
      await wait(() => expect(buttonIn(pane, "Restore").disabled).toBe(false));
    },
    TIMEOUT,
  );
});

/* ------------------------------------------------------------- study profile */

function studyWorkspace(): Workspace {
  const workspace = workspaceFixture();
  workspace.academicProfile.admissionYear = 2026;
  const enrollments: StudyEnrollment[] = [
    {
      stage: "undergraduate",
      entryKind: "regular",
      entryGrade: 1,
      admittedTerm: "2026-1Q",
    },
  ];
  workspace.academicProfile.studyEnrollments = enrollments;
  return workspace;
}

const stageSelect = (pane: HTMLElement) =>
  pane.querySelector<HTMLButtonElement>(
    '[role="combobox"][aria-label="Degree program"]',
  )!;

const yearSelect = (pane: HTMLElement) =>
  pane.querySelector<HTMLButtonElement>(
    '[data-setting="admission-year"] [role="combobox"]',
  )!;

const departmentSelect = (pane: HTMLElement) =>
  pane.querySelector<HTMLButtonElement>(
    '[data-setting="department"] [role="combobox"]',
  )!;

/** The syllabus departments as Core's finder page names them, in English
 * (the suite's content language). */
const finderDepartments = () =>
  (
    JSON.parse(
      readRepoFile("crates/app-core/src/course_query/departments.json"),
    ) as { id: string; name: { en: string } }[]
  ).map((department) => ({ ...department, name: department.name.en }));

/** Commits the study profile and the settings the way Core stores them. */
const publishEdits: Apply = (intent, core) => {
  const next = structuredClone(core.workspace);
  if (intent.kind === "setStudyProfile") {
    next.academicProfile.admissionYear = intent.admissionYear ?? undefined;
    next.academicProfile.studyEnrollments = intent.studyEnrollments;
  } else if (intent.kind === "replaceSettings") {
    next.settings = intent.settings as Workspace["settings"];
  } else return;
  core.publish(next, `revision:${core.intents.length}`);
};

async function mountStudy(
  workspace: Workspace,
  options: { handle?: Handler; apply?: Apply } = {},
): Promise<MountedApp> {
  const mounted = await mountSettings(paths.settings("study"), {
    workspace,
    handle: options.handle,
    apply: options.apply,
    finder: () => ({
      context: { departments: finderDepartments() },
      reads: ["settings"],
      seq: 0,
    }),
  });
  app = mounted;
  await wait(() => {
    if (!stageSelect(settingsPane(mounted)))
      throw new Error("The study stages are not ready.");
  });
  return mounted;
}

function studyIntents(mounted: MountedApp) {
  return mounted.core.intents.filter(
    (intent) => intent.kind === "setStudyProfile",
  );
}

/** Every study-profile write the page sent to Core, accepted or not. */
const writes = (mounted: MountedApp) =>
  mounted.core
    .commands("mutate_workspace")
    .filter(
      ({ args }) =>
        (args.mutation as { intent: { kind: string } }).intent.kind ===
        "setStudyProfile",
    ).length;

/** Chooses once the editor is free again after the previous save. */
async function chooseWhenFree(
  select: () => HTMLButtonElement,
  value: string,
): Promise<void> {
  await wait(() => expect(select().disabled).toBe(false));
  await chooseOption(select(), value);
}

/** Stores a profile edit made on another device, tells the shell, and waits
 * until it has read the workspace again. */
async function changeElsewhere(
  mounted: MountedApp,
  edit: (workspace: Workspace) => void,
): Promise<void> {
  const next = structuredClone(mounted.core.workspace);
  edit(next);
  mounted.core.publish(next, `revision:elsewhere-${mounted.core.seq}`);
  const reads = () =>
    mounted.core
      .commands("schedule_query")
      .filter(({ args }) =>
        (args.queries as { kind: string }[]).some(
          (query) => query.kind === "records",
        ),
      ).length;
  const before = reads();
  await act(async () =>
    emitNative(mounted.core, "workspace-changed", {
      seq: mounted.core.commit().seq,
      source: "cloud-sync",
      changes: ["settings"],
    }),
  );
  await wait(() => expect(reads()).toBeGreaterThan(before));
  // Let the answer reach the page.
  await advance(100);
}

describe("Study profile", () => {
  it(
    "offers the detected entry year without saving it, then saves each choice at once from what is stored",
    async () => {
      const workspace = studyWorkspace();
      delete workspace.academicProfile.admissionYear;
      workspace.academicProfile.studyEnrollments = [];
      const mounted = await mountStudy(workspace, {
        apply: publishEdits,
        handle: (command) =>
          command === "school_credential_status"
            ? { ...matrixStatus(), suggestedAdmissionYear: 2025 }
            : undefined,
      });
      const pane = settingsPane(mounted);
      const profile = () => mounted.core.workspace.academicProfile;

      // A detected year is a suggestion: nothing is stored until it is chosen.
      const offer = await wait(() =>
        buttonIn(pane, "Use 2025 from your school account"),
      );
      expect(writes(mounted)).toBe(0);
      await click(offer);
      await wait(() => expect(profile().admissionYear).toBe(2025));

      // A programme is admitted in the first quarter of the entry year.
      await chooseWhenFree(() => stageSelect(pane), "undergraduate");
      await wait(() =>
        expect(profile().studyEnrollments).toEqual([
          {
            stage: "undergraduate",
            entryKind: "unknown",
            admittedTerm: "2025-1Q",
          },
        ]),
      );

      // Each later choice changes only itself.
      await chooseWhenFree(() => yearSelect(pane), "2024");
      await wait(() => expect(profile().admissionYear).toBe(2024));
      await chooseWhenFree(() => stageSelect(pane), "professional");
      await wait(() =>
        expect(profile().studyEnrollments).toEqual([
          {
            stage: "professional",
            entryKind: "unknown",
            admittedTerm: "2025-1Q",
          },
        ]),
      );
      expect(profile().admissionYear).toBe(2024);

      // The department is a setting of its own and keeps the profile.
      await chooseOption(departmentSelect(pane), "cs");
      await wait(() =>
        expect(mounted.core.workspace.settings.departmentId).toBe("cs"),
      );
      expect(profile().admissionYear).toBe(2024);
    },
    TIMEOUT,
  );

  it(
    "locks the stage editors while a save is in flight and starts again from what is stored when it is refused",
    async () => {
      let refuse!: () => void;
      let held = true;
      const mounted = await mountStudy(studyWorkspace(), {
        apply: publishEdits,
        // The host answers the first save only when the test lets it, which
        // is the only way to observe a real mutation in flight, and then
        // refuses it. Later saves commit.
        handle: (command) => {
          if (command !== "mutate_workspace" || !held) return undefined;
          held = false;
          return new Promise((_, reject) => {
            refuse = () => reject(new Error("duplicate term"));
          });
        },
      });
      const pane = settingsPane(mounted);

      await chooseOption(stageSelect(pane), "doctoral");
      await wait(() => expect(stageSelect(pane).disabled).toBe(true));
      expect(yearSelect(pane).disabled).toBe(true);

      await act(async () => refuse());
      // The refused programme did not stay: the next choice builds on the
      // stored one.
      await chooseWhenFree(() => yearSelect(pane), "2025");
      await wait(() => expect(studyIntents(mounted)).toHaveLength(1));
      expect(studyIntents(mounted)[0]).toEqual({
        kind: "setStudyProfile",
        admissionYear: 2025,
        studyEnrollments: [
          {
            stage: "undergraduate",
            entryKind: "regular",
            entryGrade: 1,
            admittedTerm: "2026-1Q",
          },
        ],
      });
    },
    TIMEOUT,
  );

  it(
    "never overwrites a profile changed elsewhere and builds on one that arrives after a save",
    async () => {
      const initial = studyWorkspace();
      delete initial.academicProfile.admissionYear;
      initial.academicProfile.studyEnrollments = [];
      const mounted = await mountStudy(initial, { apply: publishEdits });
      const pane = settingsPane(mounted);
      await wait(() =>
        expect(mounted.core.listenerCount("workspace-changed")).toBe(1),
      );

      // Without a year the programme cannot be stored yet; it waits.
      await chooseOption(stageSelect(pane), "doctoral");
      await flush();
      expect(writes(mounted)).toBe(0);

      // Another device changes the profile while the programme waits.
      // Choosing the year then writes nothing; the page takes the newer one.
      await changeElsewhere(mounted, (next) => {
        next.academicProfile.studyEnrollments = [
          {
            stage: "master",
            entryKind: "regular",
            entryGrade: 1,
            admittedTerm: "2024-1Q",
          },
        ];
      });
      await chooseOption(yearSelect(pane), "2025");
      await flush();
      expect(writes(mounted)).toBe(0);
      await chooseWhenFree(() => yearSelect(pane), "2024");
      await wait(() => expect(studyIntents(mounted)).toHaveLength(1));
      expect(studyIntents(mounted)[0]).toEqual({
        kind: "setStudyProfile",
        admissionYear: 2024,
        studyEnrollments: [
          {
            stage: "master",
            entryKind: "regular",
            entryGrade: 1,
            admittedTerm: "2024-1Q",
          },
        ],
      });

      // A newer profile arriving after that save is what the next choice
      // builds on, not the page's own last save.
      await wait(() => expect(stageSelect(pane).disabled).toBe(false));
      await changeElsewhere(mounted, (next) => {
        next.academicProfile.studyEnrollments = [
          {
            stage: "doctoral",
            entryKind: "regular",
            entryGrade: 2,
            admittedTerm: "2026-1Q",
          },
        ];
      });
      await chooseWhenFree(() => yearSelect(pane), "2023");
      await wait(() => expect(studyIntents(mounted)).toHaveLength(2));
      expect(studyIntents(mounted)[1]).toEqual({
        kind: "setStudyProfile",
        admissionYear: 2023,
        studyEnrollments: [
          {
            stage: "doctoral",
            entryKind: "regular",
            entryGrade: 2,
            admittedTerm: "2026-1Q",
          },
        ],
      });
    },
    TIMEOUT,
  );
});

/* ------------------------------------------------------------- agent access */

describe("Agent access", () => {
  it(
    "keeps the token out of the page and copies it, alone or in each client's setup, only on click",
    async () => {
      const url = "http://127.0.0.1:47832/mcp";
      const token = "private-test-token";
      // jsdom has no clipboard; the desktop webview does.
      const copy = vi.fn(async (_text: string) => undefined);
      Object.defineProperty(navigator, "clipboard", {
        configurable: true,
        value: { writeText: copy },
      });
      const workspace = workspaceFixture();
      workspace.settings.agentAccess = { enabled: true };
      const mounted = await mountSettings(paths.settings("agent"), {
        workspace,
        handle: (command) =>
          command === "agent_connection"
            ? { state: "listening", url, token }
            : undefined,
      });
      app = mounted;
      const pane = settingsPane(mounted);

      const copyToken = await wait(() => buttonIn(pane, "Copy token"));
      expect(document.body.innerHTML).not.toContain(token);
      expect(copy).not.toHaveBeenCalled();
      await click(copyToken);
      await wait(() => expect(copy).toHaveBeenCalledTimes(1));
      expect(copy).toHaveBeenCalledWith(token);

      // Each client's setup carries the address and the whole token.
      const snippet = () => pane.querySelector<HTMLElement>(".agentSnippet")!;
      await click(buttonIn(snippet(), "Copy"));
      await wait(() => expect(copy).toHaveBeenCalledTimes(2));
      const claude = copy.mock.calls[1][0];
      expect(claude).toContain(url);
      expect(claude).toContain(token);
      await click(
        [...pane.querySelectorAll("button")].find(
          (label) => label.textContent === "Codex",
        )!,
      );
      await click(buttonIn(snippet(), "Copy"));
      await wait(() => expect(copy).toHaveBeenCalledTimes(3));
      const codex = copy.mock.calls[2][0];
      expect(codex).not.toBe(claude);
      expect(codex).toContain(url);
      expect(codex).toContain(token);
      expect(document.body.innerHTML).not.toContain(token);
    },
    TIMEOUT,
  );
});

describe("Academic calendar profile", () => {
  it(
    "saves custom class times, keeps them when the save is refused, and restores official times",
    async () => {
      const workspace = workspaceFixture();
      workspace.academicProfile.studyEnrollments = [
        { stage: "master", entryKind: "regular", admittedTerm: "2026-1Q" },
      ];
      let rejectSave = true;
      const mounted = await mountSettings(paths.settings("developer"), {
        workspace,
        queries: {
          // Native validation is covered in Rust. This host supplies its accepted
          // profile so this case exercises the real form and mutation wiring.
          validateAcademicProfile: (query) =>
            (query as { profile: Workspace["academicProfile"] }).profile,
        },
        handle: (command) => {
          if (command === "mutate_workspace" && rejectSave)
            throw new Error("Cannot save profile");
          return undefined;
        },
        apply: (intent, core) => {
          if (intent.kind === "editAcademicCalendar") {
            core.publish(
              {
                ...core.workspace,
                academicProfile:
                  intent.academicProfile as Workspace["academicProfile"],
              },
              `${core.revision}+saved`,
            );
          }
        },
      });
      app = mounted;
      const pane = settingsPane(mounted);
      await click(
        pane.querySelector('[data-setting="developer-mode"] [role="switch"]'),
      );
      const start = () =>
        pane.querySelector<HTMLInputElement>(
          '[aria-label="Periods 1–2 start time"]',
        )!;
      const official = (await wait(() => start())).value;
      await typeInto(start(), "07:30");
      await clickButton(pane, "Save");
      // The refusal is reported and the typed time stays for another try.
      await wait(() =>
        expect(pane.querySelector('[role="alert"]')).not.toBeNull(),
      );
      expect(start().value).toBe("07:30");
      rejectSave = false;
      await clickButton(pane, "Save");
      await wait(() =>
        expect(
          mounted.core.workspace.academicProfile.teachingPeriods?.[0].startTime,
        ).toBe("07:30"),
      );
      expect(mounted.core.workspace.academicProfile.studyEnrollments).toEqual(
        workspace.academicProfile.studyEnrollments,
      );
      // Reopening the editor reads the committed profile, not the previous draft.
      await mounted.go(paths.home());
      await mounted.go(paths.settings("developer"));
      await wait(() => expect(start()?.value).toBe("07:30"));
      await clickButton(pane, "Use official times");
      expect(start().value).toBe(official);
      await clickButton(pane, "Save");
      await wait(() =>
        expect(
          mounted.core.workspace.academicProfile.teachingPeriods,
        ).toBeNull(),
      );
    },
    TIMEOUT,
  );
});

describe("Course files", () => {
  it(
    "removes month-old workspaces only after the in-app confirmation",
    async () => {
      // The shell's window.confirm is asynchronous, so a `!confirm()` guard
      // never stopped this cleanup; the in-app dialog has to answer first.
      const mounted = await mountSettings(paths.settings("files"), {
        handle: (command) => (command === "clean_workspaces" ? 2 : undefined),
      });
      app = mounted;
      const pane = await wait(() => settingsPane(mounted));
      const remove = "Remove month-old workspaces";
      await wait(() => buttonIn(pane, remove));

      await clickButton(pane, remove);
      const dialog = await wait(() => openDialog());
      await clickButton(dialog, "Cancel");
      await wait(() =>
        expect(document.querySelector('[role="dialog"]')).toBeNull(),
      );
      expect(mounted.core.commands("clean_workspaces")).toHaveLength(0);

      await clickButton(pane, remove);
      await answerDialog("Remove");
      await wait(() =>
        expect(mounted.core.commands("clean_workspaces")).toHaveLength(1),
      );
    },
    TIMEOUT,
  );
});

describe("Notification preferences", () => {
  it("persists reminder choices and reconciles notification permission with the OS", async () => {
    const sent: unknown[] = [];
    const workspace = workspaceFixture();
    workspace.settings.notifications.localPermission = "denied";
    let notificationPermission: "granted" | "denied" = "granted";
    app = await mountSettings(paths.settings("notifications"), {
      workspace,
      capabilities: { notifications: true },
      handle: (command, args) => {
        // The system's real answer, which the stored permission must follow.
        if (
          command === "request_notification_permission" ||
          command === "notification_permission"
        )
          return notificationPermission;
        if (command === "notify") return (sent.push(args), true);
      },
      apply: (intent, core) => {
        if (intent.kind === "replaceSettings")
          core.publish(
            {
              ...core.workspace,
              settings: intent.settings as Workspace["settings"],
            },
            "revision:reminders",
          );
      },
    });
    const pane = settingsPane(app);
    // A persisted refusal must not disable reminders after the OS allows them.
    await wait(() =>
      expect(app!.core.workspace.settings.notifications.localPermission).toBe(
        "granted",
      ),
    );
    const classTime = pane.querySelector<HTMLInputElement>(
      '[data-setting="class-lead"] [role="combobox"]',
    )!;
    await chooseOption(classTime, "30");
    await wait(() =>
      expect(app!.core.workspace.settings.notifications.classLeadMinutes).toBe(
        30,
      ),
    );
    const examTime = pane.querySelector<HTMLInputElement>(
      '[data-setting="exam-lead"] [role="combobox"]',
    )!;
    await chooseOption(examTime, "1440");
    await wait(() =>
      expect(app!.core.workspace.settings.notifications.examLeadMinutes).toBe(
        1440,
      ),
    );
    const exams = pane.querySelector('[data-setting="exams"] [role="switch"]')!;
    await click(exams);
    await wait(() =>
      expect(pane.querySelector('[data-setting="exam-lead"]')).toBeNull(),
    );
    expect(app.core.workspace.settings.notifications.classes).toBe(true);
    await click(exams);
    await wait(() =>
      expect(pane.querySelector('[data-setting="exam-lead"]')).not.toBeNull(),
    );
    expect(app.core.workspace.settings.notifications.examLeadMinutes).toBe(
      1440,
    );
    const assignmentTime = pane.querySelector<HTMLInputElement>(
      '[aria-label="Assignment reminder times"]',
    )!;
    await click(assignmentTime);
    const option = await wait(() => {
      const option = [
        ...document.querySelectorAll<HTMLElement>(
          ".uiMultiOptions [aria-pressed]",
        ),
      ].find((item) => item.textContent === "3 hours before");
      if (!option) throw new Error("Reminder choice not visible");
      return option;
    });
    await click(option);
    await wait(() =>
      expect(
        app!.core.workspace.settings.notifications.assignmentLeadMinutes,
      ).toEqual([1440, 60, 180]),
    );
    const assignments = pane.querySelector(
      '[data-setting="assignments"] [role="switch"]',
    )!;
    await click(assignments);
    await wait(() =>
      expect(pane.querySelector('[data-setting="assignment-lead"]')).toBeNull(),
    );
    await click(assignments);
    await wait(() =>
      expect(
        pane.querySelector('[data-setting="assignment-lead"]'),
      ).not.toBeNull(),
    );
    expect(
      app.core.workspace.settings.notifications.assignmentLeadMinutes,
    ).toEqual([1440, 60, 180]);

    // A test notification asks the system first; a refusal sends nothing.
    notificationPermission = "denied";
    await click(
      pane.querySelector<HTMLButtonElement>(
        '[data-setting="test-notification"] button',
      )!,
    );
    await wait(() =>
      expect(app!.core.workspace.settings.notifications.localPermission).toBe(
        "denied",
      ),
    );
    expect(sent).toEqual([]);
    await wait(() =>
      expect(app!.core.commands("open_notification_settings")).toHaveLength(1),
    );

    // Returning from System Settings restores permission without another send.
    notificationPermission = "granted";
    await act(async () => window.dispatchEvent(new Event("focus")));
    await wait(() =>
      expect(app!.core.workspace.settings.notifications.localPermission).toBe(
        "granted",
      ),
    );
    expect(sent).toEqual([]);
    await click(
      pane.querySelector<HTMLButtonElement>(
        '[data-setting="test-notification"] button',
      )!,
    );
    await wait(() => expect(sent).toHaveLength(1));
  });
});

describe("About page", () => {
  it(
    "opens each of its links in the browser",
    async () => {
      app = await mountSettings(paths.about());
      const opened = new Set<unknown>();
      for (const row of ["website", "github", "license", "sponsor"]) {
        const button = await wait(() => {
          const found = app!.host.querySelector<HTMLButtonElement>(
            `button[data-setting="${row}"]`,
          );
          expect(found).not.toBeNull();
          return found!;
        });
        await click(button);
        await wait(() =>
          expect(app!.core.commands("open_external")).toHaveLength(
            opened.size + 1,
          ),
        );
        const { url } = app.core.commands("open_external").at(-1)!.args;
        expect(url).toMatch(/^https:\/\//);
        opened.add(url);
      }
      // Each row has its own destination.
      expect(opened.size).toBe(4);
    },
    TIMEOUT,
  );
});

describe("Campus VPN", () => {
  it(
    "connects again after a tunnel failed, though the helper still reports it",
    async () => {
      window.__ISCT_COMPANION_NATIVE_OS__ = "windows";
      const status = {
        address: null,
        expiresAt: null,
      };
      const failed = {
        ...status,
        state: "error",
        error: "tunnel_failed",
        detail: "the tunnel ended",
      };
      try {
        // What a Windows binary reports: no campus Wi-Fi sign-in.
        const mounted = await mountSettings(paths.tools(), {
          tools: ["campus-vpn", "campus-printer"],
          handle: (command) => {
            if (command === "plugin:campus-vpn|status") return failed;
            if (command === "plugin:campus-vpn|connect")
              return {
                ...status,
                state: "connected",
                error: null,
                detail: null,
              };
            return undefined;
          },
        });
        app = mounted;
        const pane = mounted.host.querySelector<HTMLElement>(".page.tools")!;
        await click(await wait(() => buttonIn(pane, "Connect")));
        await wait(() =>
          expect(
            mounted.core.commands("plugin:campus-vpn|connect"),
          ).toHaveLength(1),
        );
      } finally {
        delete window.__ISCT_COMPANION_NATIVE_OS__;
      }
    },
    TIMEOUT,
  );

  // A macOS helper waits for the person to allow it in Login Items. Connect
  // must ask for that again rather than spend a one-time APM login on a
  // helper that cannot start the tunnel, and connect once it is allowed.
  it(
    "asks for the helper's approval before it signs in",
    async () => {
      // Left to the default OS: the page has no platform branch for this,
      // and a macOS binary also starts its widget links, which the scripted
      // host does not serve.
      const status = { address: null, expiresAt: null, keepUntil: null };
      const waiting = {
        ...status,
        state: "error",
        error: "helper_needs_approval",
        detail: "the campus VPN helper waits to be allowed in System Settings",
      };
      let allowed = false;
      const mounted = await mountSettings(paths.tools(), {
        tools: ["campus-vpn"],
        handle: (command) => {
          if (command === "plugin:campus-vpn|status") return waiting;
          if (command === "plugin:campus-vpn|install")
            return allowed
              ? {
                  ...status,
                  state: "disconnected",
                  error: null,
                  detail: null,
                }
              : waiting;
          if (command === "plugin:campus-vpn|connect")
            return {
              ...status,
              state: "connected",
              error: null,
              detail: null,
            };
          return undefined;
        },
      });
      app = mounted;
      const pane = mounted.host.querySelector<HTMLElement>(".page.tools")!;
      await click(await wait(() => buttonIn(pane, "Allow…")));
      await wait(() =>
        expect(mounted.core.commands("plugin:campus-vpn|install")).toHaveLength(
          1,
        ),
      );
      expect(mounted.core.commands("plugin:campus-vpn|connect")).toHaveLength(
        0,
      );
      allowed = true;
      await clickButton(pane, "Allow…");
      await click(await wait(() => buttonIn(pane, "Connect")));
      await wait(() =>
        expect(mounted.core.commands("plugin:campus-vpn|connect")).toHaveLength(
          1,
        ),
      );
    },
    TIMEOUT,
  );

  // A macOS daemon registered from a copy of the app that is gone is
  // enabled yet never runs. Repair re-registers it; until the helper is up,
  // no login is spent on Connect.
  it(
    "repairs a helper macOS does not start before it offers Connect",
    async () => {
      const status = { address: null, expiresAt: null };
      const notStarting = {
        ...status,
        state: "error",
        error: "helper_not_starting",
        detail:
          "the campus VPN helper is registered but the system does not start it",
      };
      let repaired = false;
      const mounted = await mountSettings(paths.tools(), {
        tools: ["campus-vpn"],
        handle: (command) => {
          if (command === "plugin:campus-vpn|status") return notStarting;
          if (command === "plugin:campus-vpn|install")
            return repaired
              ? { ...status, state: "disconnected", error: null, detail: null }
              : notStarting;
          return undefined;
        },
      });
      app = mounted;
      const pane = mounted.host.querySelector<HTMLElement>(".page.tools")!;
      await click(await wait(() => buttonIn(pane, "Repair")));
      await answerDialog("Repair");
      await wait(() =>
        expect(mounted.core.commands("plugin:campus-vpn|install")).toHaveLength(
          1,
        ),
      );
      repaired = true;
      await wait(() => expect(buttonIn(pane, "Repair").disabled).toBe(false));
      await clickButton(pane, "Repair");
      await answerDialog("Repair");
      await wait(() => buttonIn(pane, "Connect"));
      expect(mounted.core.commands("plugin:campus-vpn|install")).toHaveLength(
        2,
      );
      expect(mounted.core.commands("plugin:campus-vpn|connect")).toHaveLength(
        0,
      );
    },
    TIMEOUT,
  );
});

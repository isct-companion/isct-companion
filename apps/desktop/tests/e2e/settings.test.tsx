// @vitest-environment jsdom
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  buttonByText,
  click,
  chooseOption,
  typeInto,
  courseFor,
  offering,
  flush,
  advance,
  DEFAULT_TIMETABLE_AXIS,
  mountApp,
  overviewFor,
  planningWeekFor,
  scheduleViewFor,
  until,
  workspaceFixture,
  YEAR,
  type FakeCore,
  type FakeCoreOptions,
  type MountedApp,
} from "@tests/harness";
import { DEFAULT_TEACHING_PERIODS } from "@app/generated/persistence";
import type {
  CloudSyncState,
  SchoolCredentialStatus,
} from "@app/platform/desktop-bridge";
import type { StudyEnrollment, Workspace } from "@app/lib/model";
import { paths } from "@app/app/routes";
import { schoolSessions } from "@app/platform/school-session";

/**
 * The settings pages through the real shell: school accounts and the Matrix
 * editor, backup and cloud sync, the study profile, and local agent access.
 *
 * Everything runs over the scripted host installed by `mountApp`, so the real
 * bridge, query cache and workspace store drive the real pages. Platform seams
 * are stubbed only where jsdom cannot supply the native runtime:
 *
 * - `isTauri` is set the way the native bootstrap sets it, because
 *   `recognizeMatrixImage` refuses to run without it.
 * - `navigator.clipboard` is absent in jsdom, so the agent suite installs one.
 * - ZXing camera/canvas decoding is replaced by controlled results; account
 *   selection, draft replacement and explicit credential saves remain real.
 *
 * `mountApp` must stay the first thing that imports the shell: `app/native.ts`
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
  (globalThis as unknown as Record<string, unknown>).isTauri = true;
});

afterEach(async () => {
  await app?.unmount();
  app = undefined;
  delete (globalThis as unknown as Record<string, unknown>).isTauri;
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

/** The host starts every suite with the reminder bootstrap reporting success. */
function withDefaults(handler?: Handler): Handler {
  return (command, args, core) => {
    if (command === "bootstrap_reminders") return true;
    return handler?.(command, args, core);
  };
}

function mountSettings(
  route: string,
  options: {
    workspace?: Workspace;
    capabilities?: Record<string, unknown>;
    handle?: Handler;
    apply?: Apply;
    queries?: FakeCoreOptions["queries"];
  } = {},
): Promise<MountedApp> {
  return mountApp(
    {
      workspace: options.workspace ?? workspaceFixture(),
      capabilities: options.capabilities ?? {},
      queries: {
        workspaceOverview: (_query, core) => overviewFor(core.workspace),
        planningWeek: (_query, core) => planningWeekFor(core.workspace),
        scheduleView: (_query, core) => scheduleViewFor(core.workspace),
        academicYears: { currentYear: YEAR },
        ...options.queries,
      },
      handle: withDefaults(options.handle),
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
  states: { science: "pending", tokyoTech: "pending" },
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

function cardButtons(card: HTMLElement): (string | undefined)[] {
  return [...card.querySelectorAll("button")].map((element) =>
    element.textContent?.trim(),
  );
}

describe("School accounts", () => {
  it.each([false, true])(
    "checks school OTP enrollment and gates an existing registration behind confirmation (%s)",
    async (registered) => {
      const url = registered
        ? "https://isct.ex-tic.com/idm/user/mfa_setting/"
        : "https://isct.ex-tic.com/idm/user/totp_setting/add";
      const mounted = await mountAccounts({
        capabilities: { schoolWeb: true },
        handle: (command, args) => {
          if (command === "school_otp_setup") return { registered, url };
          if (
            [
              "configure_school_web_policy",
              "set_school_web_bounds",
              "hide_school_web",
              "set_school_web_zoom",
            ].includes(command)
          )
            return true;
          if (command === "resolve_school_url")
            return { site: "science-portal", url: args.url };
          if (command === "open_school_site")
            return { site: "science-portal", url, phase: "pageLoaded" };
          return undefined;
        },
      });
      await clickButton(account(mounted, "science"), "Help me set up");
      if (registered) {
        const dialog = await wait(() => openDialog());
        expect(dialog.textContent).toContain("every other authenticator");
        expect(mounted.core.commands("resolve_school_url")).toHaveLength(0);
        await answerDialog("Cancel");
        expect(mounted.core.commands("resolve_school_url")).toHaveLength(0);
        await clickButton(account(mounted, "science"), "Help me set up");
        await answerDialog("Continue to reset");
      }
      await wait(() =>
        expect(
          mounted.core.commands("resolve_school_url").at(-1)?.args,
        ).toMatchObject({ url }),
      );
      expect(mounted.core.commands("save_school_credentials")).toHaveLength(0);
      expect(mounted.core.commands("delete_school_credentials")).toHaveLength(
        0,
      );
    },
    TIMEOUT,
  );

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
          if (command === "save_school_credentials") return true;
          if (command === "test_school_connection")
            return {
              site: "science-portal",
              url: "https://example.test",
              phase: "login",
            };
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
      await scanFile(science, PNG());
      await wait(() =>
        expect(document.body.textContent).toContain(
          "Choose your school account",
        ),
      );
      expect(mounted.core.commands("save_school_credentials")).toHaveLength(0);
      await clickButton(document.body, "School · School account");
      expect(
        science.querySelector<HTMLInputElement>("#school-otp-setup")!.value,
      ).toBe(setup);
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
      expect(mounted.core.commands("save_school_credentials")).toHaveLength(0);
      await clickButton(science, "Connect");
      await wait(() =>
        expect(
          mounted.core.commands("save_school_credentials")[0].args,
        ).toMatchObject({ credentials: { otpSecret: setup } }),
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
      await clickButton(account(mounted, "science"), "Scan QR");
      await wait(() => expect(qrVideo).toHaveBeenCalled());
      const close = document.querySelector<HTMLButtonElement>(
        '.mantine-Modal-root button[aria-label="Close"]',
      )!;
      await click(close);
      const stop = vi.fn();
      await act(async () => resolveCamera({ stop }));
      await wait(() => expect(stop).toHaveBeenCalled());
      expect(mounted.core.commands("save_school_credentials")).toHaveLength(0);
    },
    TIMEOUT,
  );

  it(
    "offers Connect and Forget on a saved portal and Not set up on an empty one",
    async () => {
      const status: SchoolCredentialStatus = {
        science: saved({ username: "saved-id", hasOtp: true }),
        tokyoTech: notConfigured(),
      };
      const mounted = await mountAccounts({
        handle: (command) => {
          if (command === "school_credential_status") return status;
          if (command === "school_session_snapshot") return noSession;
          return undefined;
        },
      });

      const science = account(mounted, "science");
      expect(
        cardButtons(science.querySelector(".schoolAccountActions")!),
      ).toEqual(["Connect", "Forget"]);
      // Saved, but no session has been reported yet in this launch.
      expect(science.querySelector(".mantine-Badge-root")).toBeNull();
      expect(
        [
          ...settingsPane(mounted).querySelectorAll<HTMLElement>(
            "section.schoolAccount",
          ),
        ].map((card) => card.dataset.setting),
      ).toEqual(["science", "tokyo-tech"]);

      const tokyoTech = account(mounted, "tokyo-tech");
      await wait(() => expect(tokyoTech.textContent).toContain("Not set up"));
      expect(cardButtons(tokyoTech)).not.toContain("Forget");
    },
    TIMEOUT,
  );

  it(
    "saves the portal credentials before it starts the tested login",
    async () => {
      const status: SchoolCredentialStatus = {
        science: saved({ username: "saved-id", hasOtp: true }),
        tokyoTech: notConfigured(),
      };
      const mounted = await mountAccounts({
        handle: (command) => {
          if (command === "school_credential_status") return status;
          if (command === "school_session_snapshot") return noSession;
          if (command === "save_school_credentials") return true;
          if (command === "test_school_connection")
            return {
              site: "science-portal",
              url: "https://example.test",
              phase: "login",
            };
          return undefined;
        },
      });

      await clickButton(account(mounted, "science"), "Connect");
      await wait(() =>
        expect(mounted.core.commands("test_school_connection")).toHaveLength(1),
      );

      const saves = mounted.core.commands("save_school_credentials");
      expect(saves).toHaveLength(1);
      expect(saves[0].args).toMatchObject({
        profile: "science",
        credentials: { username: "saved-id" },
      });
      expect(
        mounted.core.commands("test_school_connection")[0].args,
      ).toMatchObject({
        profile: "science",
        credentials: { username: "saved-id" },
      });

      const order = mounted.core.calls.map((call) => call.command);
      expect(order.indexOf("save_school_credentials")).toBeLessThan(
        order.indexOf("test_school_connection"),
      );
    },
    TIMEOUT,
  );

  it(
    "labels the login steps ID, Password, Code, Signed in and follows a native step",
    async () => {
      const status: SchoolCredentialStatus = {
        science: saved({ username: "saved-id", hasOtp: true }),
        tokyoTech: notConfigured(),
      };
      const mounted = await mountAccounts({
        handle: (command) => {
          if (command === "school_credential_status") return status;
          if (command === "school_session_snapshot") return noSession;
          if (command === "save_school_credentials") return true;
          if (command === "test_school_connection")
            return {
              site: "science-portal",
              url: "https://example.test",
              phase: "login",
            };
          return undefined;
        },
      });

      const science = account(mounted, "science");
      await clickButton(science, "Connect");
      const steps = await wait(() => {
        const element = science.querySelector(".signInProgress");
        if (!element) throw new Error("The login steps are not shown yet.");
        return element;
      });
      for (const label of ["ID", "Password", "Code", "Signed in"])
        expect(steps.textContent).toContain(label);

      // Every account card listens for its own profile; the status bar reads
      // sign-in from Core's school status instead.
      await wait(() =>
        expect(mounted.core.listenerCount("school-login-status")).toBe(2),
      );
      await act(async () =>
        emitNative(mounted.core, "school-login-status", {
          profile: "science",
          state: "progress",
          message: "otp",
        }),
      );
      await wait(() =>
        expect(
          science
            .querySelector(".schoolConnection")
            ?.getAttribute("data-state"),
        ).toBe("progress"),
      );
    },
    TIMEOUT,
  );

  it(
    "shows and updates the sign-in badge from native session events",
    async () => {
      const status: SchoolCredentialStatus = {
        science: saved({ username: "saved-id", hasOtp: true }),
        tokyoTech: notConfigured(),
      };
      const mounted = await mountAccounts({
        handle: (command) => {
          if (command === "school_credential_status") return status;
          if (command === "school_session_snapshot") return noSession;
          return undefined;
        },
      });
      const science = account(mounted, "science");
      await wait(() =>
        expect(mounted.core.listenerCount("school-session")).toBe(1),
      );

      await act(async () =>
        emitNative(mounted.core, "school-session", {
          profile: "science",
          state: "active",
          revision: 1,
        }),
      );
      await wait(() => expect(science.textContent).toContain("Signed in"));

      await act(async () =>
        emitNative(mounted.core, "school-session", {
          profile: "science",
          state: "failed",
          revision: 2,
        }),
      );
      await wait(() => expect(science.textContent).toContain("Sign-in failed"));
    },
    TIMEOUT,
  );

  it(
    "confirms before forgetting the saved portal and reports the result",
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

      await clickButton(account(mounted, "science"), "Forget");
      const confirm = await wait(() => openDialog());
      expect(confirm.textContent).toContain(
        "Forget saved Science Tokyo Portal credentials on this device?",
      );
      // Asking is not doing.
      expect(mounted.core.commands("delete_school_credentials")).toHaveLength(
        0,
      );

      await clickButton(confirm, "Forget");
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
      await wait(() =>
        expect(document.body.textContent).toContain(
          "Science Tokyo Portal account forgotten.",
        ),
      );
    },
    TIMEOUT,
  );

  it(
    "opens the login page only for a user-required event from the account being connected",
    async () => {
      const status: SchoolCredentialStatus = {
        science: saved({ username: "saved-id", hasOtp: true }),
        tokyoTech: notConfigured(),
      };
      const mounted = await mountAccounts({
        handle: (command) => {
          if (command === "school_credential_status") return status;
          if (command === "school_session_snapshot") return noSession;
          if (command === "save_school_credentials") return true;
          if (command === "test_school_connection")
            return {
              site: "science-portal",
              url: "https://example.test",
              phase: "login",
            };
          return undefined;
        },
      });

      await clickButton(account(mounted, "science"), "Connect");
      await wait(() =>
        expect(mounted.core.listenerCount("school-login-status")).toBe(2),
      );

      // The other card never started a login, so its event must not pull anyone away.
      await act(async () =>
        emitNative(mounted.core, "school-login-status", {
          profile: "tokyo-tech",
          state: "needs-user",
          message: "Complete sign in",
        }),
      );
      expect(
        sessionStorage.getItem("isct-companion-active-school-site"),
      ).toBeNull();

      await act(async () =>
        emitNative(mounted.core, "school-login-status", {
          profile: "science",
          state: "needs-user",
          message: "Complete sign in",
        }),
      );
      await wait(() =>
        expect(
          sessionStorage.getItem("isct-companion-active-school-site"),
        ).toBe("science-portal"),
      );
    },
    TIMEOUT,
  );
});

/* -------------------------------------------------------------- matrix input */

const recognitions = (text: string, confidence: number) =>
  Array.from({ length: 70 }, () => ({ text, confidence }));

const PNG = () => new File(["test"], "card.png", { type: "image/png" });

const matrixCard = (mounted: MountedApp) => account(mounted, "tokyo-tech");

function matrixInputs(card: HTMLElement): HTMLInputElement[] {
  return [...card.querySelectorAll<HTMLInputElement>(".matrixGrid input")];
}

function matrixInput(card: HTMLElement, index: number): HTMLInputElement {
  const inputs = matrixInputs(card);
  if (!inputs[index]) throw new Error(`Matrix cell ${index} is not mounted.`);
  return inputs[index];
}

async function chooseFile(input: HTMLInputElement, file: File): Promise<void> {
  Object.defineProperty(input, "files", { configurable: true, value: [file] });
  await act(async () =>
    input.dispatchEvent(new Event("change", { bubbles: true })),
  );
}

async function scanFile(card: HTMLElement, file: File): Promise<void> {
  const input = card.querySelector<HTMLInputElement>('input[type="file"]');
  if (!input) throw new Error("The Matrix scan input is not mounted.");
  await chooseFile(input, file);
}

/** A saved Tokyo Tech portal whose Matrix stays hidden until a replacement. */
const matrixStatus = (
  overrides: Partial<Portal> = {},
): SchoolCredentialStatus => ({
  science: saved({ username: "saved-id", hasOtp: true }),
  tokyoTech: saved({ username: "id", hasMatrix: true, ...overrides }),
});

describe("Matrix scan and paste", () => {
  it(
    "keeps an empty camera result retryable and closes after readable cells arrive",
    async () => {
      // Camera/decoder/canvas are hardware ports without a jsdom implementation.
      // The real modal, capture loop, native transport and editor stay mounted.
      const original = Object.getOwnPropertyDescriptor(
        navigator,
        "mediaDevices",
      );
      const stop = vi.fn();
      const track = Object.assign(new EventTarget(), { stop });
      Object.defineProperty(navigator, "mediaDevices", {
        configurable: true,
        value: {
          getUserMedia: async () => ({
            getTracks: () => [track],
            getVideoTracks: () => [track],
          }),
        },
      });
      vi.spyOn(HTMLMediaElement.prototype, "play").mockResolvedValue();
      vi.spyOn(HTMLMediaElement.prototype, "readyState", "get").mockReturnValue(
        2,
      );
      vi.spyOn(
        HTMLMediaElement.prototype,
        "currentTime",
        "get",
      ).mockImplementation(() => performance.now());
      vi.spyOn(HTMLVideoElement.prototype, "videoWidth", "get").mockReturnValue(
        1920,
      );
      vi.spyOn(
        HTMLVideoElement.prototype,
        "videoHeight",
        "get",
      ).mockReturnValue(1080);
      vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({
        drawImage: vi.fn(),
      } as unknown as CanvasRenderingContext2D);
      vi.spyOn(HTMLCanvasElement.prototype, "toDataURL").mockReturnValue(
        "data:image/jpeg;base64,Y2FtZXJh",
      );
      let finishCapture: (() => void) | undefined;
      let finishScan: (() => void) | undefined;
      let frames = 0;
      let sessions = 0;
      let processingAttempts = 0;
      const captured = {
        phase: "processing",
        readyCells: 70,
        captureElapsedMs: 800,
        timedOut: false,
        completedSamples: 0,
        totalSamples: 210,
        cells: [],
        bounds: [
          { x: 0.1, y: 0.2 },
          { x: 0.9, y: 0.2 },
          { x: 0.9, y: 0.8 },
          { x: 0.1, y: 0.8 },
        ],
      };
      try {
        const mounted = await mountAccounts({
          capabilities: { matrixOcr: true },
          handle: (command) => {
            if (command === "school_credential_status")
              return { science: notConfigured(), tokyoTech: notConfigured() };
            if (command === "school_session_snapshot") return noSession;
            if (command === "begin_matrix_camera") {
              frames = 0;
              return `camera-${++sessions}`;
            }
            if (command === "capture_matrix_camera_frame") {
              if (frames++ === 0)
                return { ...captured, phase: "capturing", readyCells: 69 };
              return new Promise((resolve) => {
                finishCapture = () => resolve(captured);
              });
            }
            if (command === "process_matrix_camera") {
              const attempt = processingAttempts++;
              return new Promise((resolve) => {
                finishScan = () =>
                  resolve({
                    ...captured,
                    phase: "complete",
                    completedSamples: 210,
                    cells: Array.from({ length: 70 }, (_, i) => ({
                      text: attempt === 0 ? "" : "A",
                      confidence: attempt === 0 ? 0 : 0.98,
                      stable: i !== 69,
                    })),
                  });
              });
            }
            return undefined;
          },
        });
        const card = matrixCard(mounted);
        await wait(() =>
          expect(cardButtons(card)).toContain("Scan with camera"),
        );
        await click(buttonIn(card, "Scan with camera"));
        await wait(() =>
          expect(
            document.body.querySelectorAll(".matrixCameraPreview polygon"),
          ).toHaveLength(1),
        );
        expect(
          document.body
            .querySelector(".matrixCameraPreview polygon")
            ?.getAttribute("points"),
        ).toBe("0.1,0.2 0.9,0.2 0.9,0.8 0.1,0.8");
        expect(
          document.body.querySelector(".matrixCameraPreview text"),
        ).toBeNull();
        expect(document.body.textContent).not.toContain(
          "Use current result and review",
        );
        expect(document.body.textContent).toContain(
          "Capturing Matrix… 69/70 cells ready.",
        );
        expect(matrixInputs(card).every((input) => input.value === "")).toBe(
          true,
        );
        await wait(() => expect(finishCapture).toBeDefined());
        await act(async () => finishCapture?.());
        await wait(() =>
          expect(document.body.textContent).toContain(
            "Processing Matrix… You can put your card down.",
          ),
        );
        // Let the real modal finish hiding while native recognition is pending.
        // A detached video ref must not cancel that work and reopen capture.
        await advance(500);
        expect(mounted.core.commands("begin_matrix_camera")).toHaveLength(1);
        expect(document.body.textContent).toContain(
          "Processing Matrix… You can put your card down.",
        );
        expect(stop).toHaveBeenCalledTimes(1);
        expect(matrixInputs(card).every((input) => input.value === "")).toBe(
          true,
        );
        expect(mounted.core.commands("stop_matrix_camera")).toHaveLength(0);
        await wait(() => expect(finishScan).toBeDefined());
        await act(async () => finishScan?.());
        await wait(() =>
          expect(document.body.textContent).toContain(
            "No readable letters found. Check for blur or covered cells and try again.",
          ),
        );
        expect(document.body.textContent).toContain(
          "Processing Matrix… You can put your card down.",
        );
        expect(document.body.textContent).not.toContain(
          "Capturing Matrix… 70/70 cells ready.",
        );
        expect(matrixInputs(card).every((input) => input.value === "")).toBe(
          true,
        );
        expect(stop).toHaveBeenCalledTimes(1);
        expect(mounted.core.commands("stop_matrix_camera")).toHaveLength(1);

        finishCapture = undefined;
        finishScan = undefined;
        await click(
          document.querySelector<HTMLButtonElement>(
            ".matrixCameraProcessing button",
          )!,
        );
        await wait(() =>
          expect(document.body.textContent).toContain(
            "Capturing Matrix… 69/70 cells ready.",
          ),
        );
        await wait(() => expect(finishCapture).toBeDefined());
        await act(async () => finishCapture?.());
        await wait(() => expect(finishScan).toBeDefined());
        await act(async () => finishScan?.());
        await wait(() =>
          expect(
            document.body.querySelector(".matrixCameraPreview"),
          ).toBeNull(),
        );
        expect(matrixInputs(card).every((input) => input.value === "A")).toBe(
          true,
        );
        expect(matrixInputs(card)[0].dataset.confidence).toBe("high");
        expect(matrixInputs(card)[69].dataset.confidence).toBe("medium");
        expect(stop).toHaveBeenCalledTimes(2);
        expect(mounted.core.commands("stop_matrix_camera")).toHaveLength(2);
      } finally {
        await app?.unmount();
        app = undefined;
        vi.restoreAllMocks();
        if (original)
          Object.defineProperty(navigator, "mediaDevices", original);
        else Reflect.deleteProperty(navigator, "mediaDevices");
      }
    },
    TIMEOUT,
  );

  it(
    "hides image scanning when this build cannot read Matrix photographs",
    async () => {
      const mounted = await mountAccounts({
        capabilities: { matrixOcr: false },
        handle: (command) => {
          if (command === "school_credential_status")
            return { science: notConfigured(), tokyoTech: notConfigured() };
          if (command === "school_session_snapshot") return noSession;
          return undefined;
        },
      });
      const pane = settingsPane(mounted);
      // The capability is read once at launch; wait for that read to settle so
      // the absence below is about the resolved capability, not a pending one.
      await wait(() =>
        expect(
          mounted.core.commands("runtime_capabilities").length,
        ).toBeGreaterThan(0),
      );
      await flush();
      expect(cardButtons(matrixCard(mounted))).not.toContain("Scan image");
      expect(cardButtons(matrixCard(mounted))).not.toContain(
        "Scan with camera",
      );
      expect(
        matrixCard(mounted).querySelector('input[type="file"]'),
      ).toBeNull();
      // QR reading is independent of native Matrix OCR availability.
      expect(
        account(mounted, "science").querySelector('input[type="file"]'),
      ).not.toBeNull();
    },
    TIMEOUT,
  );

  it(
    "shows scan confidence in three bands and masks the Matrix after Connect saves",
    async () => {
      const status: SchoolCredentialStatus = {
        science: saved({ username: "saved-id", hasOtp: true }),
        tokyoTech: notConfigured(),
      };
      const mounted = await mountAccounts({
        capabilities: { matrixOcr: true },
        handle: (command) => {
          if (command === "school_credential_status") return status;
          if (command === "school_session_snapshot") return noSession;
          if (command === "recognize_matrix_image")
            return Array.from({ length: 70 }, (_, index) => ({
              text: "A",
              confidence: index === 0 ? 0.96 : index === 1 ? 0.82 : 0.7,
            }));
          if (command === "save_school_credentials") return true;
          if (command === "test_school_connection")
            return {
              site: "tokyo-portal",
              url: "https://example.test",
              phase: "login",
            };
          return undefined;
        },
      });

      const card = matrixCard(mounted);
      await wait(() => {
        if (!card.querySelector('input[type="file"]'))
          throw new Error("The Matrix scan input is not ready.");
      });
      await scanFile(card, PNG());
      await wait(() => expect(matrixInput(card, 0).value).toBe("A"));
      expect(
        matrixInputs(card)
          .slice(0, 3)
          .map((input) => input.dataset.confidence),
      ).toEqual(["high", "medium", "low"]);

      // Saving stores the Matrix, so the same card now reports it as hidden.
      status.tokyoTech = saved({ username: "id", hasMatrix: true });
      await clickButton(card, "Connect");
      await wait(() =>
        expect(mounted.core.commands("save_school_credentials")).toHaveLength(
          1,
        ),
      );
      expect(
        mounted.core.commands("save_school_credentials")[0].args,
      ).toMatchObject({
        profile: "tokyo-tech",
        credentials: { matrix: "A".repeat(70) },
      });
      expect(
        mounted.core.commands("test_school_connection").length,
      ).toBeGreaterThan(0);

      // A saved Matrix keeps its 7×10 shape with every letter and confidence
      // hidden; typing over a cell is how it is replaced.
      await wait(() =>
        expect(
          matrixInputs(card).every(
            (input) => input.value === "" && input.placeholder === "•",
          ),
        ).toBe(true),
      );
      expect(matrixInputs(card)).toHaveLength(70);
      expect(matrixInputs(card).some((input) => input.dataset.confidence)).toBe(
        false,
      );
      expect(card.textContent).toContain("70/70");
      expect(card.textContent).not.toContain("AAAA");
      // Connect's sign-in check holds the card until the portal answers.
      await act(async () =>
        emitNative(mounted.core, "school-login-status", {
          profile: "tokyo-tech",
          state: "connected",
          message: "",
        }),
      );
      await wait(() => expect(matrixInputs(card)[0].disabled).toBe(false));
    },
    TIMEOUT,
  );

  it(
    "refuses an incomplete scan and keeps the letters it could read",
    async () => {
      const mounted = await mountAccounts({
        capabilities: { matrixOcr: true },
        handle: (command) => {
          if (command === "school_credential_status")
            return { science: notConfigured(), tokyoTech: notConfigured() };
          if (command === "school_session_snapshot") return noSession;
          if (command === "recognize_matrix_image")
            return Array.from({ length: 70 }, (_, index) => ({
              text: index === 0 ? "L" : "",
              confidence: index === 0 ? 0.92 : 0,
            }));
          return undefined;
        },
      });

      const card = matrixCard(mounted);
      await wait(() => {
        if (!card.querySelector('input[type="file"]'))
          throw new Error("The Matrix scan input is not ready.");
      });
      await scanFile(card, PNG());
      await wait(() => expect(matrixInput(card, 0).value).toBe("L"));

      await clickButton(card, "Connect");
      await wait(() =>
        expect(document.body.textContent).toContain("Fill all 70 matrix cells"),
      );
      expect(mounted.core.commands("save_school_credentials")).toHaveLength(0);
      // The readable cell survives the refusal.
      expect(matrixInput(card, 0).value).toBe("L");
      expect(
        card.querySelectorAll('[data-confidence="low"]').length,
      ).toBeGreaterThanOrEqual(69);
    },
    TIMEOUT,
  );

  it(
    "keeps the scanned letters and their confidence when saving fails",
    async () => {
      const mounted = await mountAccounts({
        capabilities: { matrixOcr: true },
        handle: (command) => {
          if (command === "school_credential_status")
            return { science: notConfigured(), tokyoTech: notConfigured() };
          if (command === "school_session_snapshot") return noSession;
          if (command === "recognize_matrix_image")
            return recognitions("C", 0.94);
          if (command === "save_school_credentials")
            throw new Error("Vault unavailable");
          return undefined;
        },
      });

      const card = matrixCard(mounted);
      await wait(() => {
        if (!card.querySelector('input[type="file"]'))
          throw new Error("The Matrix scan input is not ready.");
      });
      await scanFile(card, PNG());
      await wait(() => expect(matrixInput(card, 0).value).toBe("C"));

      await clickButton(card, "Connect");
      await wait(() => expect(card.textContent).toContain("Vault unavailable"));
      expect(matrixInput(card, 0).value).toBe("C");
      expect(matrixInput(card, 0).dataset.confidence).toBe("high");
    },
    TIMEOUT,
  );

  it.each(["success", "empty", "error"] as const)(
    "resets the file input after a %s scan so the same photo can be chosen again",
    async (outcome) => {
      let scanned = 0;
      const mounted = await mountAccounts({
        capabilities: { matrixOcr: true },
        handle: (command) => {
          if (command === "school_credential_status")
            return { science: notConfigured(), tokyoTech: notConfigured() };
          if (command === "school_session_snapshot") return noSession;
          if (command === "recognize_matrix_image") {
            scanned += 1;
            if (scanned === 1) {
              if (outcome === "error") throw new Error("Scan failed");
              return recognitions(outcome === "empty" ? "" : "A", 0.95);
            }
            return recognitions("B", 0.95);
          }
          return undefined;
        },
      });

      const card = matrixCard(mounted);
      const input = await wait(() => {
        const element =
          card.querySelector<HTMLInputElement>('input[type="file"]');
        if (!element) throw new Error("The Matrix scan input is not ready.");
        return element;
      });
      // A browser only re-fires `change` for the same file once the value is
      // cleared, so the suite pretends the picker put a path there.
      let selected = "";
      Object.defineProperty(input, "value", {
        configurable: true,
        get: () => selected,
        set: (value: string) => {
          selected = value;
        },
      });

      selected = "C:\\fakepath\\card.png";
      await chooseFile(input, PNG());
      await wait(() => expect(scanned).toBe(1));
      await wait(() => expect(selected).toBe(""));
      await wait(() =>
        expect(buttonIn(card, "Scan image").disabled).toBe(false),
      );

      selected = "C:\\fakepath\\card.png";
      await chooseFile(input, PNG());
      await wait(() => expect(scanned).toBe(2));
      await wait(() => expect(matrixInput(card, 0).value).toBe("B"));
      expect(selected).toBe("");
    },
    TIMEOUT,
  );

  it(
    "pastes whitespace-formatted letters into the right grid cells",
    async () => {
      const mounted = await mountAccounts({
        capabilities: { matrixOcr: true },
        handle: (command) => {
          if (command === "school_credential_status")
            return { science: notConfigured(), tokyoTech: notConfigured() };
          if (command === "school_session_snapshot") return noSession;
          return undefined;
        },
      });
      const card = matrixCard(mounted);
      const inputs = await wait(() => {
        const all = matrixInputs(card);
        if (all.length !== 70) throw new Error("The Matrix grid is not ready.");
        return all;
      });

      const letters = Array.from({ length: 70 }, (_, index) =>
        String.fromCharCode(65 + (index % 26)),
      );
      const text = Array.from({ length: 7 }, (_, row) =>
        letters.slice(row * 10, row * 10 + 10).join(" "),
      ).join("\n");

      const paste = new Event("paste", { bubbles: true, cancelable: true });
      Object.defineProperty(paste, "clipboardData", {
        value: { getData: () => text },
      });
      // Pasting into a middle cell must not spread the letters from there.
      await act(async () => inputs[5].dispatchEvent(paste));

      expect(matrixInputs(card).map((input) => input.value)).toEqual(letters);
    },
    TIMEOUT,
  );

  it(
    "asks before copying the saved plaintext Matrix",
    async () => {
      const status = matrixStatus();
      const mounted = await mountAccounts({
        capabilities: { matrixOcr: true },
        handle: (command) => {
          if (command === "school_credential_status") return status;
          if (command === "school_session_snapshot") return noSession;
          if (command === "copy_school_matrix") return true;
          return undefined;
        },
      });
      const card = matrixCard(mounted);
      await wait(() => {
        if (!buttonByText(card, "Copy"))
          throw new Error("The Copy button is not ready.");
      });

      await clickButton(card, "Copy");
      const confirm = await wait(() => openDialog());
      expect(confirm.textContent).toContain(
        "Copy the saved Matrix as plain text?",
      );
      await clickButton(confirm, "Cancel");
      await wait(() =>
        expect(document.querySelector('[role="dialog"]')).toBeNull(),
      );
      expect(mounted.core.commands("copy_school_matrix")).toHaveLength(0);

      await clickButton(card, "Copy");
      await answerDialog("Copy");
      await wait(() =>
        expect(mounted.core.commands("copy_school_matrix")).toHaveLength(1),
      );
      expect(buttonIn(card, "Copy")).toBeTruthy();
    },
    TIMEOUT,
  );
});

it(
  "renders bounded virtual application logs, pauses following while reading older entries, and exports them",
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
        '[data-setting="developer-mode"] input',
      )!,
    );
    await click(toggle);
    await app.go(paths.settings("diagnostics"));
    await wait(() =>
      expect(settingsPane(app!).textContent).toContain("1500 log entries"),
    );
    const pane = settingsPane(app);
    await wait(() =>
      expect(pane.querySelectorAll(".diagnosticLogRow").length).toBeGreaterThan(
        0,
      ),
    );
    expect(pane.querySelectorAll(".diagnosticLogRow").length).toBeLessThan(40);
    expect(pane.querySelector('[contenteditable="true"],textarea')).toBeNull();
    const follow = pane.querySelector<HTMLInputElement>(
      'input[type="checkbox"]',
    )!;
    expect(follow.checked).toBe(true);
    await act(async () =>
      pane
        .querySelector(".diagnosticLogs")!
        .dispatchEvent(
          new WheelEvent("wheel", { deltaY: -100, bubbles: true }),
        ),
    );
    expect(follow.checked).toBe(false);
    await click(follow);
    expect(follow.checked).toBe(true);

    await clickButton(pane, "Save to file");
    await wait(() => expect(app!.core.commands("save_file")).toHaveLength(1));
    const saved = app.core.commands("save_file")[0].args;
    expect(saved.name).toMatch(/^IsctCompanion-logs-\d{4}-\d{2}-\d{2}\.txt$/);
    expect(saved.contents).toBe("exported log text");
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

async function mountBackup(
  options: { handle?: Handler } = {},
): Promise<MountedApp> {
  const mounted = await mountSettings(paths.settings("backup"), {
    capabilities: { cloudSync: true },
    handle: options.handle,
  });
  app = mounted;
  await wait(() =>
    expect(settingsPane(mounted).textContent).toContain("iCloud"),
  );
  return mounted;
}

const driveToggle = (pane: HTMLElement) =>
  pane.querySelector<HTMLInputElement>(
    '[data-setting="cloud"] input[type="checkbox"]',
  )!;

const autoToggle = (pane: HTMLElement) =>
  pane.querySelector<HTMLInputElement>(
    '[data-setting="sync-automatic"] input',
  )!;

function passwordFields(pane: HTMLElement): NodeListOf<HTMLInputElement> {
  return pane.querySelectorAll<HTMLInputElement>('input[type="password"]');
}

describe("Backup", () => {
  it(
    "turns Google Drive on before offering the encrypted credential flow",
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
      // Drive is off, so its encrypted credential flow stays hidden.
      expect(pane.querySelector('[data-setting="credential-sync"]')).toBeNull();

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

      await wait(() =>
        expect(
          pane.querySelector('[data-setting="credential-sync"]'),
        ).not.toBeNull(),
      );
      // Off: no password fields until Set up is chosen.
      expect(passwordFields(pane)).toHaveLength(0);
      await wait(() => {
        if (!buttonByText(pane, "Set up"))
          throw new Error("Set up is not offered yet.");
      });
      await clickButton(pane, "Set up");
      expect(passwordFields(pane)).toHaveLength(2);
      expect(pane.textContent).toContain(
        "Encrypted with this password before upload.",
      );
      await clickButton(pane, "Cancel");
      expect(passwordFields(pane)).toHaveLength(0);
    },
    TIMEOUT,
  );

  it(
    "uploads, pulls, and restores a kept version only after confirming",
    async () => {
      const mounted = await mountBackup({
        handle: (command) => {
          if (command === "cloud_sync_state")
            return cloudState({
              recoveryPoints: Array.from({ length: 9 }, (_, index) => ({
                id: `local:${index}`,
                version: `v1-${index}`,
                savedAt: `2026-10-${String(12 - index).padStart(2, "0")}T03:00:00Z`,
              })),
            });
          if (command === "sync_cloud_now") return cloudState();
          if (command === "restore_cloud_version") return cloudState();
          return undefined;
        },
      });
      const pane = settingsPane(mounted);

      const visibleExportRows = () =>
        [...pane.querySelectorAll("button")]
          .filter((button) => button.textContent === "Export…")
          .map((button) => button.closest(".row")?.textContent);
      const firstPageRows = visibleExportRows();
      expect(
        [...pane.querySelectorAll("button")].filter(
          (button) => button.textContent === "Export…",
        ),
      ).toHaveLength(5); // four recovery files plus manual export
      const localPages = pane.querySelector(
        '[aria-label="Local recovery pages"]',
      )!;
      await click(
        localPages.querySelector<HTMLButtonElement>('[aria-label="2"]') ??
          [...localPages.querySelectorAll<HTMLButtonElement>("button")].find(
            (button) => button.textContent === "2",
          )!,
      );
      expect(
        [...pane.querySelectorAll("button")].filter(
          (button) => button.textContent === "Export…",
        ),
      ).toHaveLength(5);
      expect(visibleExportRows()).not.toEqual(firstPageRows);
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
      expect(confirm.textContent).toContain("Restore the version from");
      expect(mounted.core.commands("restore_cloud_version")).toHaveLength(0);

      await clickButton(confirm, "Restore");
      await wait(() =>
        expect(mounted.core.commands("restore_cloud_version")).toHaveLength(1),
      );
      expect(mounted.core.commands("restore_cloud_version")[0].args).toEqual({
        id: "v1",
      });
      await wait(() =>
        expect(document.body.textContent).toContain(
          "Restored the earlier version.",
        ),
      );
    },
    TIMEOUT,
  );

  it(
    "switches Auto explicitly and shows unfinished provider deliveries",
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

      await wait(() =>
        expect(pane.textContent).toContain("Saved on this device"),
      );
      // Transport attempts belong in application logs, not the backup history.
      expect(pane.textContent).not.toContain("Waiting to retry");
      expect(pane.textContent).not.toContain("Attempts");
      expect(pane.textContent).not.toContain("task-1");
      expect(pane.textContent).not.toContain("Up to date");
      expect(buttonIn(pane, "Restore").disabled).toBe(true);

      // A transfer Core reports as running replaces the waiting label.
      await act(async () =>
        emitNative(
          mounted.core,
          "cloud-sync-state",
          cloudState({ automatic: true, syncing: true }),
        ),
      );
      await wait(() => expect(pane.textContent).toContain("Syncing…"));
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
  pane.querySelector<HTMLInputElement>(
    '[role="combobox"][aria-label="Degree programme"]',
  )!;

const departmentSelect = (pane: HTMLElement) =>
  pane.querySelector<HTMLInputElement>(
    '[data-setting="department"] [role="combobox"]',
  )!;

async function mountStudy(
  workspace: Workspace,
  options: { handle?: Handler; apply?: Apply } = {},
): Promise<MountedApp> {
  const mounted = await mountSettings(paths.settings("study"), {
    workspace,
    handle: options.handle,
    apply: options.apply,
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

describe("Study profile", () => {
  it("persists a department selection after the controlled field rerenders", async () => {
    const mounted = await mountStudy(studyWorkspace(), {
      apply: (intent, core) => {
        if (intent.kind === "replaceSettings")
          core.publish(
            {
              ...core.workspace,
              settings: intent.settings as Workspace["settings"],
            },
            "revision:profile-edit",
          );
      },
    });
    const pane = settingsPane(mounted);
    await chooseOption(departmentSelect(pane), "cs");
    await wait(() =>
      expect(mounted.core.workspace.settings.departmentId).toBe("cs"),
    );
    expect(departmentSelect(pane).value).toBe("Computer Science");
  });

  it("prefills the detected entry year and saves it only on request", async () => {
    const workspace = studyWorkspace();
    delete workspace.academicProfile.admissionYear;
    workspace.academicProfile.studyEnrollments = [];
    const mounted = await mountStudy(workspace, {
      apply: (intent, core) => {
        if (intent.kind === "setStudyProfile")
          core.publish(
            {
              ...core.workspace,
              academicProfile: {
                ...core.workspace.academicProfile,
                admissionYear: intent.admissionYear ?? undefined,
                studyEnrollments: intent.studyEnrollments,
              },
            },
            "revision:profile-edit",
          );
      },
      handle: (command) => {
        if (command === "school_credential_status")
          return { ...matrixStatus(), suggestedAdmissionYear: 2025 };
      },
    });
    const pane = settingsPane(mounted);
    await wait(() =>
      expect(
        pane.querySelector<HTMLInputElement>(
          '[data-setting="admission-year"] [role="combobox"]',
        )!.value,
      ).toBe("2025 entry"),
    );
    expect(studyIntents(mounted)).toHaveLength(0);
    await chooseOption(stageSelect(pane), "undergraduate");
    await clickButton(pane, "Save enrollment details");
    await wait(() =>
      expect(mounted.core.workspace.academicProfile.admissionYear).toBe(2025),
    );
    expect(mounted.core.workspace.academicProfile.studyEnrollments).toEqual([
      { stage: "undergraduate", entryKind: "unknown", admittedTerm: "2025-1Q" },
    ]);
  });

  it("keeps a manually saved admission year when a different year is detected", async () => {
    const workspace = studyWorkspace();
    workspace.academicProfile.admissionYear = 2024;
    const mounted = await mountStudy(workspace, {
      handle: (command) => {
        if (command === "school_credential_status")
          return { ...matrixStatus(), suggestedAdmissionYear: 2025 };
      },
    });
    const pane = settingsPane(mounted);
    await flush();
    expect(
      pane.querySelector<HTMLInputElement>(
        '[data-setting="admission-year"] [role="combobox"]',
      )!.value,
    ).toBe("2024 entry");
    expect(buttonIn(pane, "Save enrollment details").disabled).toBe(true);
    expect(studyIntents(mounted)).toHaveLength(0);
  });

  it(
    "saves the simple year and degree fields without discarding known enrollment details",
    async () => {
      const mounted = await mountStudy(studyWorkspace());
      const pane = settingsPane(mounted);
      expect(pane.querySelectorAll('[role="combobox"]')).toHaveLength(3);
      expect(pane.textContent).not.toContain("Add degree programme");
      await chooseOption(stageSelect(pane), "professional");
      await chooseOption(
        pane.querySelector<HTMLElement>(
          '[data-setting="admission-year"] [role="combobox"]',
        )!,
        "2025",
      );

      await clickButton(pane, "Save enrollment details");
      await wait(() => expect(studyIntents(mounted)).toHaveLength(1));
      // One save carries every edit, so a field cannot be lost half-way.
      expect(studyIntents(mounted)[0]).toEqual({
        kind: "setStudyProfile",
        admissionYear: 2025,
        studyEnrollments: [
          {
            stage: "professional",
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
    "locks the stage editors while a save is in flight",
    async () => {
      let release!: () => void;
      const pending = new Promise<void>((resolve) => {
        release = resolve;
      });
      const mounted = await mountStudy(studyWorkspace(), {
        // The host answers only when the test lets it, which is the only way to
        // observe the in-flight state of a real mutation.
        handle: (command, _args, core) => {
          if (command !== "mutate_workspace") return undefined;
          return pending.then(() => ({
            revision: `${core.revision}+host`,
            contentRevision: `${core.revision}+host`,
            workspace: structuredClone(core.workspace),
            schoolRecords: structuredClone(core.schoolRecords),
          }));
        },
      });
      const pane = settingsPane(mounted);

      await chooseOption(stageSelect(pane), "doctoral");
      await clickButton(pane, "Save enrollment details");
      await wait(() => expect(stageSelect(pane).disabled).toBe(true));
      expect(
        pane.querySelector<HTMLInputElement>(
          '[data-setting="admission-year"] [role="combobox"]',
        )!.disabled,
      ).toBe(true);
      expect(buttonIn(pane, "Saving…").disabled).toBe(true);

      await act(async () => release());
      await wait(() => expect(stageSelect(pane).disabled).toBe(false));
    },
    TIMEOUT,
  );

  it(
    "keeps a rejected draft visible for correction",
    async () => {
      const mounted = await mountStudy(studyWorkspace(), {
        handle: (command) => {
          if (command === "mutate_workspace") throw new Error("duplicate term");
          return undefined;
        },
      });
      const pane = settingsPane(mounted);

      await chooseOption(stageSelect(pane), "doctoral");
      await clickButton(pane, "Save enrollment details");
      const error = await wait(() => {
        const alert = pane.querySelector('[role="alert"]');
        if (!alert) throw new Error("The rejection is not reported yet.");
        return alert;
      });
      expect(error.textContent).toContain("duplicate term");
      expect(stageSelect(pane).value).toBe("Doctoral");
      expect(buttonIn(pane, "Save enrollment details").disabled).toBe(false);
    },
    TIMEOUT,
  );

  it(
    "does not overwrite an external profile change with a dirty draft",
    async () => {
      const initial = studyWorkspace();
      const mounted = await mountStudy(initial);
      const pane = settingsPane(mounted);

      await chooseOption(stageSelect(pane), "doctoral");

      // Another device changed the profile while this draft was open.
      const external = structuredClone(initial);
      external.academicProfile.studyEnrollments![0].entryGrade = 2;
      external.settings.departmentId = "physics";
      mounted.core.publish(external, "revision:external");
      await wait(() =>
        expect(mounted.core.listenerCount("workspace-changed")).toBe(1),
      );
      await act(async () =>
        emitNative(mounted.core, "workspace-changed", {
          source: "cloud-sync",
          changes: ["settings"],
        }),
      );
      // The department is not part of the draft, so it proves the newer profile
      // has really been adopted before Save is pressed.
      await wait(() => expect(departmentSelect(pane).value).toBe("Physics"));

      await clickButton(pane, "Save enrollment details");
      const error = await wait(() => {
        const alert = pane.querySelector('[role="alert"]');
        if (!alert) throw new Error("The conflict is not reported yet.");
        return alert;
      });
      expect(error.textContent).toContain("changed elsewhere");
      expect(stageSelect(pane).value).toBe("Doctoral");
      expect(studyIntents(mounted)).toHaveLength(0);
    },
    TIMEOUT,
  );

  it(
    "adopts a newer external profile after a successful save",
    async () => {
      const initial = studyWorkspace();
      const mounted = await mountStudy(initial, {
        apply: (intent, core) => {
          if (intent.kind !== "setStudyProfile") return;
          const next = structuredClone(core.workspace);
          next.academicProfile.admissionYear =
            intent.admissionYear ?? undefined;
          next.academicProfile.studyEnrollments = intent.studyEnrollments;
          core.publish(next, "revision:saved");
        },
      });
      const pane = settingsPane(mounted);

      await chooseOption(stageSelect(pane), "master");
      await clickButton(pane, "Save enrollment details");
      await wait(() =>
        expect(buttonIn(pane, "Save enrollment details").disabled).toBe(true),
      );
      expect(stageSelect(pane).value).toBe("Master");

      const external = structuredClone(initial);
      external.academicProfile.studyEnrollments = [
        {
          stage: "doctoral",
          entryKind: "regular",
          entryGrade: 2,
          admittedTerm: "2026-1Q",
        },
      ];
      mounted.core.publish(external, "revision:external");
      await wait(() =>
        expect(mounted.core.listenerCount("workspace-changed")).toBe(1),
      );
      await act(async () =>
        emitNative(mounted.core, "workspace-changed", {
          source: "cloud-sync",
          changes: ["settings"],
        }),
      );
      await wait(() => expect(stageSelect(pane).value).toBe("Doctoral"));
    },
    TIMEOUT,
  );
});

/* ------------------------------------------------------------- agent access */

async function mountAgent(
  enabled: boolean,
  handle: Handler,
): Promise<MountedApp> {
  const workspace = workspaceFixture();
  workspace.settings.agentAccess = { enabled, lastWrites: [] };
  const mounted = await mountSettings(paths.settings("agent"), {
    workspace,
    handle,
  });
  app = mounted;
  return mounted;
}

describe("Agent access", () => {
  it(
    "shows the HTTP address and copies the token only on click",
    async () => {
      const url = "http://127.0.0.1:47832/mcp";
      const token = "private-test-token";
      // jsdom has no clipboard; the desktop webview does.
      const copy = vi.fn(async () => undefined);
      Object.defineProperty(navigator, "clipboard", {
        configurable: true,
        value: { writeText: copy },
      });
      const mounted = await mountAgent(true, (command) =>
        command === "agent_connection"
          ? { state: "listening", url, token }
          : undefined,
      );
      const pane = settingsPane(mounted);

      await wait(() => expect(pane.textContent).toContain(url));
      expect(pane.querySelector("code")?.textContent).toBe(url);
      expect(pane.textContent).not.toContain("--bridge");
      expect(document.body.innerHTML).not.toContain(token);
      expect(copy).not.toHaveBeenCalled();

      const copyToken = buttonIn(pane, "Copy token");
      await click(copyToken);
      await wait(() => expect(copy).toHaveBeenCalledTimes(1));
      expect(copy).toHaveBeenCalledWith(token);
    },
    TIMEOUT,
  );

  it(
    "reports a server startup failure instead of a nonworking address",
    async () => {
      const mounted = await mountAgent(true, (command) =>
        command === "agent_connection"
          ? { state: "failed", error: "The MCP port 47832 is already in use." }
          : undefined,
      );
      const pane = settingsPane(mounted);

      const alert = await wait(() => {
        const element = pane.querySelector('[role="alert"]');
        if (!element)
          throw new Error("The startup failure is not reported yet.");
        return element;
      });
      expect(alert.textContent).toContain("already in use");
      expect(pane.querySelector("code")).toBeNull();
      expect(pane.textContent).not.toContain("Copy token");
    },
    TIMEOUT,
  );

  it(
    "offers no address or token while agent access is off",
    async () => {
      const mounted = await mountAgent(false, (command) =>
        command === "agent_connection" ? { state: "off" } : undefined,
      );
      const pane = settingsPane(mounted);

      await wait(() =>
        expect(pane.textContent).toContain("Allow local agents"),
      );
      expect(pane.textContent).not.toContain("Connect an agent");
      expect(pane.querySelector("code")).toBeNull();
      expect(pane.querySelector('[role="alert"]')).toBeNull();
      expect(pane.textContent).not.toContain("Copy token");
    },
    TIMEOUT,
  );
});

describe("Academic calendar profile", () => {
  it(
    "saves custom class times, uses them on the view axis, and restores official times",
    async () => {
      const course = courseFor(offering("clock", "CSC.T271", "Clock course"));
      const workspace = workspaceFixture({ courses: [course] });
      workspace.academicProfile.studyEnrollments = [
        { stage: "master", entryKind: "regular", admittedTerm: "2026-1Q" },
      ];
      let rejectSave = true;
      const mounted = await mountSettings(paths.settings("developer"), {
        workspace,
        queries: {
          // Native validation is covered in Rust. This host supplies its accepted
          // profile so this case exercises the real form, mutation and view wiring.
          validateAcademicProfile: (query) =>
            (query as { profile: Workspace["academicProfile"] }).profile,
          // The axis geometry is Core's (tested in Rust); this host answers
          // with the stored periods so the saved times reach the view axis.
          timetableAxis: (_query, core) => {
            const periods =
              core.workspace.academicProfile.teachingPeriods ??
              DEFAULT_TEACHING_PERIODS;
            return {
              ...DEFAULT_TIMETABLE_AXIS,
              rows: periods.map((period, index) => ({
                periods: period.periods,
                startTime: period.startTime,
                endTime: period.endTime,
                top: index * 2,
                bottom: index * 2 + 2,
              })),
            };
          },
          scheduleView: (_query, core) =>
            scheduleViewFor(core.workspace, {
              agendas: [
                {
                  date: "2026-10-12",
                  noClasses: false,
                  importantDates: [],
                  occurrences: [],
                },
              ],
            }),
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
      await click(pane.querySelector('[data-setting="developer-mode"] input'));
      const start = () =>
        pane.querySelector<HTMLInputElement>(
          '[aria-label="Periods 1–2 start time"]',
        )!;
      await wait(() => expect(start()?.value).toBe("08:50"));
      expect(pane.textContent).toContain("Using official quarter dates");
      expect(pane.textContent).not.toContain("Grade path");
      await typeInto(start(), "07:30");
      await clickButton(pane, "Save");
      await wait(() =>
        expect(pane.querySelector('[role="alert"]')?.textContent).toContain(
          "Cannot save profile",
        ),
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
      await mounted.go(paths.home("week"));
      await wait(() =>
        expect(mounted.host.querySelector(".ttAxis")?.textContent).toContain(
          "07:30",
        ),
      );
      // Reopening the editor reads the committed profile, not the previous draft.
      await mounted.go(paths.settings("developer"));
      await wait(() => expect(start()?.value).toBe("07:30"));
      await clickButton(pane, "Use official times");
      expect(start().value).toBe("08:50");
      await clickButton(pane, "Save");
      await wait(() =>
        expect(
          mounted.core.workspace.academicProfile.teachingPeriods,
        ).toBeNull(),
      );
      await mounted.go(paths.home("week"));
      await wait(() => {
        expect(mounted.host.querySelector(".ttAxis")?.textContent).toContain(
          "08:50",
        );
        expect(
          mounted.host.querySelector(".ttAxis")?.textContent,
        ).not.toContain("07:30");
      });
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
    expect(
      classTime.closest('[role="group"]')!.getAttribute("aria-label"),
    ).toBe("Class reminders");
    await chooseOption(classTime, "30");
    await wait(() =>
      expect(app!.core.workspace.settings.notifications.classLeadMinutes).toBe(
        30,
      ),
    );
    const examTime = pane.querySelector<HTMLInputElement>(
      '[data-setting="exam-lead"] [role="combobox"]',
    )!;
    expect(examTime.closest('[role="group"]')!.getAttribute("aria-label")).toBe(
      "Exam reminders",
    );
    await chooseOption(examTime, "1440");
    await wait(() =>
      expect(app!.core.workspace.settings.notifications.examLeadMinutes).toBe(
        1440,
      ),
    );
    const exams = pane.querySelector('[data-setting="exams"] input')!;
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
    expect(
      assignmentTime.closest(".settingsSuboptions")!.getAttribute("aria-label"),
    ).toBe("Assignment reminders");
    await click(assignmentTime);
    const option = await wait(() => {
      const option = [
        ...document.querySelectorAll<HTMLElement>('[role="option"]'),
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
      '[data-setting="assignments"] input',
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
    expect(sent[0]).toEqual({
      title: "IsctCompanion",
      body: "This is a test notification.",
    });
    await wait(() =>
      expect(document.body.textContent).toContain(
        "The system accepted the test notification.",
      ),
    );
  });
});

describe("desktop app updates", () => {
  it(
    "shows an unpublished stable channel without claiming the app is up to date",
    async () => {
      const workspace = workspaceFixture();
      workspace.settings.locale = "zh";
      app = await mountSettings(paths.settings("about"), {
        workspace,
        handle: (command) => {
          if (command === "check_app_update" || command === "app_update_state")
            return {
              phase: "unpublished",
              signedUpdates: true,
              releaseUrl:
                "https://github.com/isct-companion/isct-companion/releases/latest",
            };
          return undefined;
        },
      });
      await wait(() =>
        expect(app!.host.textContent).toContain("尚未发布稳定版"),
      );
      expect(app.host.textContent).not.toContain("已是最新版本");
      expect(buttonByText(app.host, "下载更新")).toBeUndefined();
      await clickButton(app.host, "检查更新");
      await wait(() =>
        expect(app!.host.textContent).toContain("尚未发布稳定版"),
      );
    },
    TIMEOUT,
  );
});

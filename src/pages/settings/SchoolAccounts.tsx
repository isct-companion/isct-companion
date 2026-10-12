import { errorText } from "../../app/shared";
import { notify as appNotify } from "../../app/notify";
import { useEffect, useRef, useState } from "react";
import {
  Badge,
  Button,
  FileButton,
  Group,
  Paper,
  Progress,
  Stack,
  Text,
  TextInput,
} from "@mantine/core";
import { UiInput } from "../../components/mantine";

import { desktopBridge } from "../../app/native";
import {
  MATRIX_COLUMNS,
  MATRIX_ROWS,
  MATRIX_SIZE,
  matrixConfidence,
  normalizeMatrixText,
  parseMatrixText,
  recognizeMatrixImage,
} from "../../core/matrix-ocr";
import type {
  SchoolAuthProfile,
  SchoolCredentialStatus,
  SchoolCredentials,
  SchoolLoginEvent,
} from "../../platform/desktop-bridge";
import {
  schoolSessions,
  useSchoolSession,
} from "../../platform/school-session";

const emptyStatus: SchoolCredentialStatus = {
  science: {
    availability: "not-configured",
    saved: false,
    username: "",
    hasOtp: false,
    hasMatrix: false,
  },
  tokyoTech: {
    availability: "not-configured",
    saved: false,
    username: "",
    hasOtp: false,
    hasMatrix: false,
  },
};

type Draft = {
  username: string;
  password: string;
  otpSecret: string;
  matrix: string[];
};

// Steps the login script reports, in the order each portal shows them.
const STEPS = [
  { key: "identifier", label: "ID" },
  { key: "password", label: "Password" },
  { key: "code", label: "Code" },
  { key: "connected", label: "Signed in" },
] as const;
// Portal-side steps that sit between the ones shown above.
const STEP_ALIASES: Record<string, string> = {
  agree: "password",
  method: "code",
  otp: "code",
  matrix: "code",
};
// Reset on every reported step; a single portal round trip is far shorter.
const STEP_TIMEOUT_MS = 30_000;

type Connection = {
  state: "idle" | "checking" | "timeout" | SchoolLoginEvent["state"];
  step: string;
  message: string;
};

function SignInSteps({ connection }: { connection: Connection }) {
  const connected = connection.state === "connected";
  const current = connected
    ? STEPS.length - 1
    : Math.max(
        0,
        STEPS.findIndex((step) => step.key === connection.step),
      );
  const failed = !["checking", "progress", "connected"].includes(
    connection.state,
  );
  const value = connected ? 100 : ((current + 0.35) / STEPS.length) * 100;
  return (
    <Stack gap={5} className="signInProgress">
      <Group justify="space-between" gap="xs" wrap="nowrap">
        {STEPS.map((step, index) => (
          <Text
            key={step.key}
            size="xs"
            fw={index === current ? 700 : 500}
            c={
              connected || index < current
                ? "green"
                : index === current && failed
                  ? "red"
                  : index === current
                    ? undefined
                    : "dimmed"
            }
          >
            {step.label}
          </Text>
        ))}
      </Group>
      <Progress
        value={value}
        size="sm"
        radius="xl"
        color={failed ? "red" : connected ? "green" : "campus"}
        animated={!failed && !connected}
      />
    </Stack>
  );
}

const emptyDraft = (): Draft => ({
  username: "",
  password: "",
  otpSecret: "",
  matrix: Array.from({ length: MATRIX_SIZE }, () => ""),
});

function MatrixEditor({
  value,
  masked,
  onBeginEdit,
  onChange,
  notify,
  disabled,
  onScanState,
  onCopy,
  matrixOcrAvailable,
}: {
  disabled: boolean;
  onScanState: (scanning: boolean) => void;
  value: string[];
  masked: boolean;
  onBeginEdit: () => void;
  onChange: (value: string[]) => void;
  notify: (message: string) => void;
  onCopy: () => void;
  matrixOcrAvailable: boolean;
}) {
  const inputs = useRef<Array<HTMLInputElement | null>>([]);
  const [recognizing, setRecognizing] = useState(false);
  const resetScanInput = useRef<(() => void) | null>(null);
  const [scanProgress, setScanProgress] = useState(0);
  const [confidence, setConfidence] = useState<Array<number | undefined>>([]);
  useEffect(() => {
    if (masked || value.every((cell) => !cell)) setConfidence([]);
  }, [masked, value]);
  const markManual = (indexes: number[]) =>
    setConfidence((current) => {
      const next = [...current];
      indexes.forEach((index) => {
        next[index] = undefined;
      });
      return next;
    });
  const write = (start: number, raw: string) => {
    onBeginEdit();
    const letters = normalizeMatrixText(raw);
    if (!letters) return;
    const next = [...value];
    [...letters].forEach((letter, offset) => {
      if (start + offset < MATRIX_SIZE) next[start + offset] = letter;
    });
    markManual([...letters].map((_, offset) => start + offset));
    onChange(next);
    inputs.current[Math.min(MATRIX_SIZE - 1, start + letters.length)]?.focus();
  };
  const load = (text: string) => {
    try {
      const cells = parseMatrixText(text);
      onBeginEdit();
      onChange(cells);
      setConfidence([]);
      notify("Matrix pasted. Verify it against your card, then Connect.");
    } catch (error) {
      appNotify.error(error);
    }
  };
  const recognize = async (file?: File) => {
    if (!file || recognizing || disabled) return;
    setRecognizing(true);
    onScanState(true);
    setScanProgress(0);
    try {
      const result = await recognizeMatrixImage(file, setScanProgress);
      if (result.every((cell) => !cell.text)) {
        notify(
          "No readable letters found. Check for blur or covered cells and try again.",
        );
        return;
      }
      onBeginEdit();
      onChange(result.map((cell) => cell.text));
      setConfidence(result.map((cell) => cell.confidence));
      notify(
        "Scan complete. Verify all letters against your card; highlighted cells need review.",
      );
    } catch (error) {
      appNotify.error(error);
    } finally {
      resetScanInput.current?.();
      setRecognizing(false);
      onScanState(false);
    }
  };
  return (
    <div className="matrixEditor">
      <div className="matrixTools">
        <b>Matrix</b>
        <span>{masked ? 70 : value.filter(Boolean).length}/70</span>
        {matrixOcrAvailable && (
          <FileButton
            resetRef={resetScanInput}
            disabled={recognizing || disabled}
            onChange={(file) => void recognize(file ?? undefined)}
            accept="image/*"
          >
            {(props) => (
              <Button
                {...props}
                variant="default"
                size="compact-xs"
                loading={recognizing}
                disabled={disabled}
              >
                {recognizing ? `Reading ${scanProgress}/70…` : "Scan image"}
              </Button>
            )}
          </FileButton>
        )}
        {masked && (
          <Button
            variant="default"
            size="compact-xs"
            disabled={recognizing || disabled}
            onClick={onCopy}
          >
            Copy
          </Button>
        )}
      </div>
      {!masked && (
        <div
          className="matrixConfidenceLegend"
          aria-label="Recognition confidence"
        >
          <span data-confidence="high">Green · High</span>
          <span data-confidence="medium">Yellow · Review</span>
          <span data-confidence="low">Red · Low / unreadable</span>
          <span>Uncolored · Manual entry</span>
        </div>
      )}
      {masked && (
        <Text size="xs" c="dimmed">
          Saved · hidden. Scan or type to replace.
        </Text>
      )}
      <div className="matrixGrid" aria-label="Tokyo Tech matrix">
        <span />
        {Array.from({ length: MATRIX_COLUMNS }, (_, column) => (
          <b key={column}>{String.fromCharCode(65 + column)}</b>
        ))}
        {Array.from({ length: MATRIX_ROWS }, (_, row) => (
          <div className="matrixRow" key={row}>
            <b>{row + 1}</b>
            {Array.from({ length: MATRIX_COLUMNS }, (_, column) => {
              const index = row * MATRIX_COLUMNS + column;
              return (
                <UiInput
                  key={column}
                  ref={(element) => {
                    inputs.current[index] = element;
                  }}
                  disabled={recognizing || disabled}
                  aria-label={`${String.fromCharCode(65 + column)}${row + 1}`}
                  data-confidence={
                    !masked && confidence[index] !== undefined
                      ? matrixConfidence(value[index], confidence[index]!)
                      : undefined
                  }
                  title={
                    masked
                      ? "Saved · hidden"
                      : confidence[index] === undefined
                        ? "Manual entry"
                        : `${matrixConfidence(value[index], confidence[index]!)} confidence · verify against card`
                  }
                  value={masked ? "" : value[index]}
                  placeholder={masked ? "•" : ""}
                  autoCapitalize="characters"
                  autoComplete="off"
                  spellCheck={false}
                  inputMode="text"
                  maxLength={1}
                  onFocus={(event) => {
                    event.currentTarget.select();
                  }}
                  onChange={(event) => {
                    onBeginEdit();
                    const letter =
                      normalizeMatrixText(event.target.value).at(-1) ?? "";
                    const next = [...value];
                    next[index] = letter;
                    onChange(next);
                    markManual([index]);
                    if (letter) inputs.current[index + 1]?.focus();
                  }}
                  onPaste={(event) => {
                    event.preventDefault();
                    const text = event.clipboardData.getData("text");
                    // A complete Matrix always replaces the grid from A1;
                    // shorter snippets continue from the focused cell.
                    if (text.replace(/\s/g, "").length >= MATRIX_SIZE)
                      load(text);
                    else write(index, text);
                  }}
                  onKeyDown={(event) => {
                    if (
                      event.key === "Backspace" &&
                      !value[index] &&
                      index > 0
                    ) {
                      event.preventDefault();
                      const next = [...value];
                      next[index - 1] = "";
                      onBeginEdit();
                      markManual([index - 1]);
                      onChange(next);
                      inputs.current[index - 1]?.focus();
                    } else if (event.key === "ArrowLeft" && index > 0) {
                      event.preventDefault();
                      inputs.current[index - 1]?.focus();
                    } else if (
                      event.key === "ArrowRight" &&
                      index + 1 < MATRIX_SIZE
                    ) {
                      event.preventDefault();
                      inputs.current[index + 1]?.focus();
                    } else if (event.key === "ArrowUp" && row > 0) {
                      event.preventDefault();
                      inputs.current[index - MATRIX_COLUMNS]?.focus();
                    } else if (
                      event.key === "ArrowDown" &&
                      row + 1 < MATRIX_ROWS
                    ) {
                      event.preventDefault();
                      inputs.current[index + MATRIX_COLUMNS]?.focus();
                    }
                  }}
                />
              );
            })}
          </div>
        ))}
      </div>
    </div>
  );
}

function Account({
  profile,
  title,
  summary,
  draft,
  setDraft,
  refresh,
  notify,
  showLoginPage,
  matrixOcrAvailable,
}: {
  profile: SchoolAuthProfile;
  title: string;
  summary: SchoolCredentialStatus["science"];
  draft: Draft;
  setDraft: (draft: Draft) => void;
  refresh: () => Promise<void>;
  notify: (message: string) => void;
  showLoginPage: (profile: SchoolAuthProfile) => void;
  matrixOcrAvailable: boolean;
}) {
  const [busy, setBusy] = useState(false);
  const [matrixReplacing, setMatrixReplacing] = useState(false);
  const [scanning, setScanning] = useState(false);
  const [connection, setConnection] = useState<Connection>({
    state: "idle",
    step: "",
    message: "",
  });
  const timeout = useRef<number | undefined>(undefined);
  // Only an attempt started from this card may pull the person to the page.
  const started = useRef(false);
  const armTimeout = () => {
    if (timeout.current) window.clearTimeout(timeout.current);
    timeout.current = window.setTimeout(() => {
      setConnection((current) => ({
        ...current,
        state: "timeout",
        message: "No response from the portal.",
      }));
      setBusy(false);
      started.current = false;
    }, STEP_TIMEOUT_MS);
  };
  useEffect(() => {
    const dispose = desktopBridge?.onSchoolLoginStatus?.((event) => {
      if (event.profile !== profile) return;
      if (event.state === "progress") {
        const step = STEP_ALIASES[event.message] ?? event.message;
        setConnection({ state: "progress", step, message: "" });
        setBusy(true);
        armTimeout();
        return;
      }
      if (timeout.current) window.clearTimeout(timeout.current);
      setConnection((current) => ({
        state: event.state,
        step: current.step,
        message: event.state === "connected" ? "" : event.message,
      }));
      setBusy(false);
      if (event.state === "needs-user" && started.current)
        showLoginPage(profile);
      started.current = false;
    });
    return () => {
      dispose?.();
      if (timeout.current) window.clearTimeout(timeout.current);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [profile]);
  const stored = useSchoolSession(desktopBridge, profile);
  // Signed in, failed, or not set up; nothing while the first result is out.
  const unavailable = summary.availability
    ? summary.availability === "not-configured" ||
      summary.availability === "missing"
    : !summary.saved;
  const session = unavailable
    ? "not-configured"
    : stored === "not-configured"
      ? undefined
      : stored;
  const credentials = (): SchoolCredentials => ({
    username: draft.username,
    password: draft.password,
    otpSecret: draft.otpSecret,
    matrix: draft.matrix.join(""),
  });
  const validateMatrix = () => {
    const count = draft.matrix.filter(Boolean).length;
    if (
      profile === "tokyo-tech" &&
      ((count > 0 && count < MATRIX_SIZE) || (matrixReplacing && count === 0))
    ) {
      notify(
        "Fill all 70 matrix cells before saving. Red or blank cells need review.",
      );
      return false;
    }
    return true;
  };
  const connect = async () => {
    if (!desktopBridge || scanning || !validateMatrix()) return;
    const nextCredentials = credentials();
    setBusy(true);
    started.current = true;
    setConnection({ state: "checking", step: "", message: "" });
    schoolSessions.set(profile, undefined);
    armTimeout();
    try {
      await desktopBridge.saveSchoolCredentials(profile, nextCredentials);
      setDraft({ ...emptyDraft(), username: draft.username.trim() });
      setMatrixReplacing(false);
      await refresh();
      await desktopBridge.testSchoolConnection(profile, nextCredentials);
    } catch (error) {
      if (timeout.current) window.clearTimeout(timeout.current);
      started.current = false;
      setConnection({
        state: "error",
        step: "",
        message:
          errorText(error) ?? "Sign-in could not start.",
      });
      setBusy(false);
      await refresh().catch(() => undefined);
    }
  };
  const copyMatrix = async () => {
    if (
      !desktopBridge ||
      !confirm(
        "Copy the saved Matrix as plain text? Other apps may be able to read the clipboard until you replace it.",
      )
    )
      return;
    try {
      if (await desktopBridge.copySchoolMatrix())
        notify("Matrix copied. Paste it soon, then replace your clipboard.");
    } catch (error) {
      // Native commands reject with their error string.
      appNotify.error(error);
    }
  };
  const forget = async () => {
    if (
      !desktopBridge ||
      !confirm(`Forget saved ${title} credentials on this device?`)
    )
      return;
    setBusy(true);
    try {
      await desktopBridge.deleteSchoolCredentials(profile);
      setDraft(emptyDraft());
      setMatrixReplacing(false);
      setConnection({ state: "idle", step: "", message: "" });
      await refresh();
      schoolSessions.set(profile, "not-configured");
      notify(`${title} account forgotten.`);
    } finally {
      setBusy(false);
    }
  };
  return (
    <Paper
      component="section"
      className="schoolAccount"
      data-setting={profile === "science" ? "science" : "tokyo-tech"}
      p="md"
      radius="md"
      withBorder
    >
      <Group component="header" justify="space-between" wrap="nowrap">
        <Text fw={700}>{title}</Text>
        {session && (
          <Badge
            variant="light"
            color={
              session === "active"
                ? "green"
                : session === "failed"
                  ? "red"
                  : "gray"
            }
          >
            {session === "active"
              ? "Signed in"
              : session === "failed"
                ? "Sign-in failed"
                : "Not set up"}
          </Badge>
        )}
      </Group>
      {connection.state !== "idle" && (
        <div className="schoolConnection" data-state={connection.state}>
          <SignInSteps connection={connection} />
          {connection.message && <span>{connection.message}</span>}
          {connection.state !== "connected" && (
            <Button
              variant="subtle"
              size="compact-xs"
              onClick={() => showLoginPage(profile)}
            >
              Show page
            </Button>
          )}
        </div>
      )}
      {summary.availability === "missing" && (
        <Text c="red" size="sm">
          Saved credentials are missing from the system credential vault. Enter
          them again.
        </Text>
      )}
      {summary.availability === "access-denied" && (
        <Text c="orange" size="sm">
          The system credential vault is unavailable. Existing credentials were
          not changed.
        </Text>
      )}
      <div className="credentialFields">
        <TextInput
          label="ID"
          value={draft.username}
          autoCapitalize="none"
          autoComplete="username"
          onChange={(event) =>
            setDraft({ ...draft, username: event.target.value })
          }
        />
        {/* A saved secret shows as fixed dots; typing replaces it, blank keeps it. */}
        <TextInput
          type="password"
          label="Password"
          value={draft.password}
          placeholder={summary.saved ? SAVED_SECRET : ""}
          autoComplete="current-password"
          onChange={(event) =>
            setDraft({ ...draft, password: event.target.value })
          }
        />
        {profile === "science" && (
          <TextInput
            type="password"
            className="wide"
            label="OTP setup link or secret"
            value={draft.otpSecret}
            autoCapitalize="none"
            autoComplete="off"
            placeholder={summary.hasOtp ? SAVED_SECRET : "otpauth://… or Base32"}
            onChange={(event) =>
              setDraft({ ...draft, otpSecret: event.target.value })
            }
          />
        )}
      </div>
      {profile === "tokyo-tech" && (
        <MatrixEditor
          disabled={busy}
          onScanState={setScanning}
          value={draft.matrix}
          masked={summary.hasMatrix && !matrixReplacing}
          onBeginEdit={() => setMatrixReplacing(true)}
          onChange={(matrix) => setDraft({ ...draft, matrix })}
          notify={notify}
          onCopy={() => void copyMatrix()}
          matrixOcrAvailable={matrixOcrAvailable}
        />
      )}
      <div className="actions schoolAccountActions">
        <Button
          disabled={busy || scanning}
          loading={busy}
          onClick={() => void connect()}
        >
          Connect
        </Button>
        {summary.saved && (
          <Button
            variant="subtle"
            color="red"
            disabled={busy || scanning}
            onClick={() => void forget()}
          >
            Forget
          </Button>
        )}
      </div>
    </Paper>
  );
}

/** What a saved password or OTP setup shows in its empty field. */
const SAVED_SECRET = "••••••••";

export function SchoolAccounts({
  notify,
  showLoginPage,
  matrixOcrAvailable,
}: {
  notify: (message: string) => void;
  showLoginPage: (profile: SchoolAuthProfile) => void;
  matrixOcrAvailable: boolean;
}) {
  const [status, setStatus] = useState(emptyStatus);
  const [science, setScience] = useState(emptyDraft);
  const [tokyoTech, setTokyoTech] = useState(emptyDraft);
  const refresh = async () => {
    if (!desktopBridge) return;
    const next = await desktopBridge.schoolCredentialStatus();
    setStatus(next);
    setScience((current) => ({
      ...current,
      username: current.username || next.science.username,
    }));
    setTokyoTech((current) => ({
      ...current,
      username: current.username || next.tokyoTech.username,
    }));
  };
  useEffect(() => {
    void refresh().catch((error) =>
      appNotify.error(error),
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  if (!desktopBridge)
    return (
      <p className="settingsNote">
        School sign-in needs the installed app, which keeps passwords in the
        system keychain.
      </p>
    );
  return (
    <div className="schoolAccounts">
      <Account
        profile="science"
        title="Science Tokyo Portal"
        summary={status.science}
        draft={science}
        setDraft={setScience}
        refresh={refresh}
        notify={notify}
        showLoginPage={showLoginPage}
        matrixOcrAvailable={matrixOcrAvailable}
      />
      <Account
        profile="tokyo-tech"
        title="Tokyo Tech Portal"
        summary={status.tokyoTech}
        draft={tokyoTech}
        setDraft={setTokyoTech}
        refresh={refresh}
        notify={notify}
        showLoginPage={showLoginPage}
        matrixOcrAvailable={matrixOcrAvailable}
      />
    </div>
  );
}

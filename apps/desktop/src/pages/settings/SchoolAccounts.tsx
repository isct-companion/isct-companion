import { errorText } from "../../app/shared";
import { notify as appNotify } from "../../app/notify";
import {
  sessionDetails,
  sessionText,
  signInProblemText,
} from "../../app/sync-text";
import { useEffect, useRef, useState } from "react";
import {
  Badge,
  Button,
  FileButton,
  Group,
  Paper,
  Text,
  TextInput,
} from "../../components/ui";
import { OtpSetupInput } from "../../components/OtpSetupInput";
import { MatrixCamera } from "../../components/MatrixCamera";
import { UiInput } from "../../components/ui";
import { confirmAction } from "../../components/confirm";
import i18n, { t, type MessageKey } from "../../i18n";

import { desktopBridge } from "../../app/native";
import {
  MATRIX_COLUMNS,
  MATRIX_ROWS,
  MATRIX_SIZE,
  matrixConfidence,
  type MatrixConfidence,
  normalizeMatrixText,
  parseMatrixText,
  recognizeMatrixImage,
} from "../../lib/matrix-ocr";
import type {
  SchoolAuthProfile,
  SchoolConnectOutcome,
  SchoolCredentialStatus,
  SchoolCredentials,
  SchoolSessionState,
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

const CONFIDENCE_NOTE = {
  high: "settings.schoolAccounts.confidenceHigh",
  medium: "settings.schoolAccounts.confidenceMedium",
  low: "settings.schoolAccounts.confidenceLow",
} satisfies Record<MatrixConfidence, MessageKey>;

const SESSION_BADGE: Record<SchoolSessionState, string> = {
  "not-configured": "gray",
  normal: "green",
  "signing-in": "campus",
  refused: "red",
  "needs-web": "yellow",
  unavailable: "orange",
};

const OUTCOME_COLOR: Record<SchoolConnectOutcome["kind"], string> = {
  connected: "green",
  refused: "red",
  "needs-web": "yellow",
  unreachable: "orange",
  "not-saved": "red",
  stopped: "dimmed",
};

/** What the form says about its last Connect. Only `connected` saved the
 * entered credentials; every other outcome says they were not saved. */
function connectOutcomeText(outcome: SchoolConnectOutcome) {
  switch (outcome.kind) {
    case "connected":
      return t("schoolSession.connect.connected");
    case "refused":
    case "needs-web":
      return [
        signInProblemText(outcome.reason),
        t("schoolSession.connect.notSaved"),
      ]
        .filter(Boolean)
        .join(" ");
    case "unreachable":
      return t("schoolSession.connect.unreachable");
    case "not-saved":
      return t("schoolSession.connect.saveFailed");
    case "stopped":
      return t("schoolSession.connect.stopped");
  }
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
  const [cameraOpen, setCameraOpen] = useState(false);
  const scanState = useRef(onScanState);
  scanState.current = onScanState;
  useEffect(() => {
    scanState.current(cameraOpen || recognizing);
    return () => scanState.current(false);
  }, [cameraOpen, recognizing]);
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
      notify(t("settings.schoolAccounts.matrixPastedVerifyItAgainst"));
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
        notify(t("matrixCamera.noReadableLettersFoundCheck"));
        return;
      }
      onBeginEdit();
      onChange(result.map((cell) => cell.text));
      setConfidence(result.map((cell) => cell.confidence));
      notify(t("settings.schoolAccounts.scanCompleteVerifyAllLetters"));
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
        <b>{t("settings.schoolAccounts.matrix")}</b>
        <span>{masked ? 70 : value.filter(Boolean).length}/70</span>
        {matrixOcrAvailable && (
          <Button
            variant="default"
            size="compact-xs"
            disabled={recognizing || disabled || cameraOpen}
            onClick={() => {
              setCameraOpen(true);
              onScanState(true);
            }}
          >
            {t("settings.schoolAccounts.scanWithCamera")}
          </Button>
        )}
        {cameraOpen && (
          <MatrixCamera
            onClose={() => {
              setCameraOpen(false);
              onScanState(false);
            }}
            onComplete={(cells) => {
              onBeginEdit();
              onChange(cells.map((cell) => cell.text));
              setConfidence(cells.map((cell) => cell.confidence));
              setCameraOpen(false);
              onScanState(false);
              notify(t("settings.schoolAccounts.scanCompleteVerifyAllLetters"));
            }}
          />
        )}
        {matrixOcrAvailable && (
          <FileButton
            resetRef={resetScanInput}
            disabled={recognizing || disabled || cameraOpen}
            onChange={(file) => void recognize(file ?? undefined)}
            accept="image/*,.heic,.heif"
          >
            {(props) => (
              <Button
                {...props}
                variant="default"
                size="compact-xs"
                loading={recognizing}
                disabled={disabled}
              >
                {recognizing
                  ? t("settings.schoolAccounts.readingCount70", {
                      count: scanProgress,
                    })
                  : t("settings.schoolAccounts.scanImage")}
              </Button>
            )}
          </FileButton>
        )}
        {masked && (
          <Button
            variant="default"
            size="compact-xs"
            disabled={recognizing || disabled || cameraOpen}
            onClick={onCopy}
          >
            {t("settings.schoolAccounts.copy")}
          </Button>
        )}
      </div>
      {!masked && (
        <div
          className="matrixConfidenceLegend"
          aria-label={t("settings.schoolAccounts.recognitionConfidence")}
        >
          <span data-confidence="high">
            {t("settings.schoolAccounts.greenHigh")}
          </span>
          <span data-confidence="medium">
            {t("settings.schoolAccounts.yellowReview")}
          </span>
          <span data-confidence="low">
            {t("settings.schoolAccounts.redLowUnreadable")}
          </span>
          <span>{t("settings.schoolAccounts.uncoloredManualEntry")}</span>
        </div>
      )}
      {/* The saved Matrix keeps its 7×10 shape; typing over a cell replaces it. */}
      <div
        className="matrixGrid"
        aria-label={t("settings.schoolAccounts.tokyoTechMatrix")}
      >
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
                  disabled={recognizing || disabled || cameraOpen}
                  aria-label={`${String.fromCharCode(65 + column)}${row + 1}`}
                  data-confidence={
                    !masked && confidence[index] !== undefined
                      ? matrixConfidence(value[index], confidence[index]!)
                      : undefined
                  }
                  title={
                    masked
                      ? t("settings.schoolAccounts.savedHidden")
                      : confidence[index] === undefined
                        ? t("settings.schoolAccounts.manualEntry")
                        : t(
                            CONFIDENCE_NOTE[
                              matrixConfidence(value[index], confidence[index]!)
                            ],
                          )
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
  showLoginPage: (
    profile: SchoolAuthProfile,
    options?: { unsigned: true },
  ) => void;
  matrixOcrAvailable: boolean;
}) {
  const [busy, setBusy] = useState(false);
  const [retrying, setRetrying] = useState(false);
  // A save that could not run at all (the vault, the command); the session
  // snapshot says nothing about it.
  const [error, setError] = useState("");
  // The last Connect's outcome, until the next edit or Connect.
  const [outcome, setOutcome] = useState<SchoolConnectOutcome | null>(null);
  const edit = (next: Draft) => {
    setOutcome(null);
    setDraft(next);
  };
  const [matrixReplacing, setMatrixReplacing] = useState(false);
  const [scanning, setScanning] = useState(false);
  // Core's session read model; nothing while the first snapshot is out.
  const session = useSchoolSession(desktopBridge, profile);
  const details = session
    ? sessionDetails(profile, session, i18n.language)
    : [];
  // Retry re-reads the synced credentials and signs in once now; Core also
  // tries an unavailable school again by itself.
  const retryOffered =
    session?.state === "refused" ||
    session?.state === "needs-web" ||
    session?.state === "unavailable";
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
      notify(t("settings.schoolAccounts.fillAll70MatrixCells"));
      return false;
    }
    return true;
  };
  const connect = async () => {
    if (!desktopBridge || scanning || !validateMatrix()) return;
    setBusy(true);
    setError("");
    setOutcome(null);
    try {
      const next = await desktopBridge.connectSchool(profile, credentials());
      // Only a connection saved the draft. Any other outcome keeps it, so
      // the person can correct it (or add the OTP setup) and connect again.
      if (next.kind === "connected") {
        setDraft({ ...emptyDraft(), username: draft.username.trim() });
        setMatrixReplacing(false);
      }
      // The card follows Core's session; the form says what this click did.
      setOutcome(next);
      await refresh();
    } catch (reason) {
      setError(
        errorText(reason) ?? t("settings.schoolAccounts.signInCouldNotStart"),
      );
      await refresh().catch(() => undefined);
    } finally {
      setBusy(false);
    }
  };
  const retry = async () => {
    if (!desktopBridge) return;
    setRetrying(true);
    try {
      await schoolSessions.retry(desktopBridge, profile);
    } catch (error) {
      appNotify.error(error);
    } finally {
      setRetrying(false);
    }
  };
  const copyMatrix = async () => {
    if (
      !desktopBridge ||
      !(await confirmAction(
        t("settings.schoolAccounts.copyTheSavedMatrixAs"),
        t("settings.schoolAccounts.copy"),
      ))
    )
      return;
    try {
      if (await desktopBridge.copySchoolMatrix())
        notify(t("settings.schoolAccounts.matrixCopiedPasteItSoon"));
    } catch (error) {
      // Native commands reject with their error string.
      appNotify.error(error);
    }
  };
  const forget = async () => {
    if (
      !desktopBridge ||
      !(await confirmAction(
        t("settings.schoolAccounts.forgetSavedTitleCredentialsOn", { title }),
        t("settings.schoolAccounts.forget"),
      ))
    )
      return;
    setBusy(true);
    try {
      await desktopBridge.deleteSchoolCredentials(profile);
      setError("");
      setOutcome(null);
      setDraft(emptyDraft());
      setMatrixReplacing(false);
      await refresh();
      notify(t("settings.schoolAccounts.accountForgotten", { account: title }));
    } catch (reason) {
      // Signing out did not finish: the account stays saved, with Forget,
      // and pressing it again redoes every step.
      setError(errorText(reason) ?? t("error.sign-out-incomplete"));
      await refresh().catch(() => undefined);
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
            color={SESSION_BADGE[session.state]}
            title={session.problem?.message}
          >
            {sessionText(session, i18n.language)}
          </Badge>
        )}
      </Group>
      {session && (details.length > 0 || retryOffered) && (
        <div
          className="schoolConnection"
          data-state={session.state}
          data-maintenance={session.maintenance ? "" : undefined}
        >
          <span>{details.join(" ")}</span>
          {session.state === "needs-web" && (
            <Button
              variant="subtle"
              size="compact-xs"
              onClick={() => showLoginPage(profile)}
            >
              {t("settings.schoolAccounts.showPage")}
            </Button>
          )}
          {retryOffered && !session.signOutPending && (
            <Button
              variant="subtle"
              size="compact-xs"
              disabled={busy || retrying}
              loading={retrying}
              onClick={() => void retry()}
            >
              {t("common.retry")}
            </Button>
          )}
        </div>
      )}
      {session?.state === "refused" && (
        <Text size="xs" c="dimmed" className="schoolRetryHint">
          {t("schoolSession.retry.otherDevice")}
        </Text>
      )}
      {error && (
        <Text c="red" size="sm" role="alert">
          {error}
        </Text>
      )}
      {summary.availability === "missing" && (
        <Text c="red" size="sm">
          {t("settings.schoolAccounts.savedCredentialsAreMissingFrom")}
        </Text>
      )}
      {summary.availability === "access-denied" && (
        <Text c="orange" size="sm">
          {t("settings.schoolAccounts.theSystemCredentialVaultIs")}
        </Text>
      )}
      <div className="credentialFields">
        <TextInput
          label={t("settings.schoolAccounts.id")}
          value={draft.username}
          autoCapitalize="none"
          autoComplete="username"
          onChange={(event) => edit({ ...draft, username: event.target.value })}
        />
        {/* A saved secret shows as fixed dots; typing replaces it, blank keeps it. */}
        <TextInput
          type="password"
          label={t("settings.main.password")}
          value={draft.password}
          placeholder={summary.saved ? SAVED_SECRET : ""}
          autoComplete="current-password"
          onChange={(event) => edit({ ...draft, password: event.target.value })}
        />
        {profile === "science" && (
          <OtpSetupInput
            value={draft.otpSecret}
            saved={summary.hasOtp}
            disabled={busy}
            onScanState={setScanning}
            onChange={(otpSecret) => edit({ ...draft, otpSecret })}
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
          onChange={(matrix) => edit({ ...draft, matrix })}
          notify={notify}
          onCopy={() => void copyMatrix()}
          matrixOcrAvailable={matrixOcrAvailable}
        />
      )}
      <div className="actions schoolAccountActions">
        <Button
          disabled={busy || scanning || session?.signOutPending}
          loading={busy}
          onClick={() => void connect()}
        >
          {t("tools.campusWifi.connect")}
        </Button>
        {(summary.saved || summary.hasOtp || session?.signOutPending) && (
          <Button
            variant="subtle"
            color="red"
            disabled={busy || scanning}
            onClick={() => void forget()}
          >
            {t("settings.schoolAccounts.forget")}
          </Button>
        )}
      </div>
      {outcome && (
        <Text size="sm" c={OUTCOME_COLOR[outcome.kind]} role="status">
          {connectOutcomeText(outcome)}
        </Text>
      )}
      {/* The app signs in to the portal by itself only with App OTP: the
          person sets it up on the portal, signed in by hand there. */}
      {profile === "science" &&
        outcome?.kind === "needs-web" &&
        outcome.reason === "email-otp" && (
          <div className="schoolConnection">
            <span>{t("schoolSession.appOtp.explain")}</span>
            <Button
              variant="subtle"
              size="compact-xs"
              onClick={() => showLoginPage(profile, { unsigned: true })}
            >
              {t("schoolSession.appOtp.openPortal")}
            </Button>
          </div>
        )}
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
  showLoginPage: (
    profile: SchoolAuthProfile,
    options?: { unsigned: true },
  ) => void;
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
    void refresh().catch((error) => appNotify.error(error));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  if (!desktopBridge)
    return (
      <p className="settingsNote">
        {t("settings.schoolAccounts.schoolSignInNeedsThe")}
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

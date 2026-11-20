import type {
  SchoolAuthProfile,
  SchoolSessionState,
  SchoolSignInProblemKind,
  SchoolSessionSnapshot,
  SignInPresentation,
} from "../school-session-contract";

import {
  quietly,
  requireString,
  type BridgeCommands,
  type ListenNative,
} from "./helpers";

export type SchoolCredentials = {
  username: string;
  password: string;
  otpSecret: string;
  matrix: string;
};
export type SchoolOtpCode = {
  otp: string;
  expiresAt: number;
  period: number;
};

export type SchoolCredentialStatus = {
  suggestedAdmissionYear?: number;
  science: {
    availability:
      | "not-configured"
      | "saved-locked"
      | "available"
      | "missing"
      | "access-denied";
    saved: boolean;
    username: string;
    hasOtp: boolean;
    hasMatrix: boolean;
  };
  tokyoTech: {
    availability:
      | "not-configured"
      | "saved-locked"
      | "available"
      | "missing"
      | "access-denied";
    saved: boolean;
    username: string;
    hasOtp: boolean;
    hasMatrix: boolean;
  };
};
const SCHOOL_SESSION_STATES: readonly unknown[] = [
  "not-configured",
  "normal",
  "signing-in",
  "refused",
  "needs-web",
  "unavailable",
] satisfies SchoolSessionState[];
/** How Connect ended. Only `connected` saved anything: the entered
 * credentials are the verified ones now and sync to the other devices. */
export type SchoolConnectOutcome = { presentation: SignInPresentation } & (
  | { kind: "connected" | "unreachable" | "not-saved" | "stopped" }
  | {
      kind: "refused" | "needs-web";
      reason: SchoolSignInProblemKind;
      refusalUnsaved: boolean;
    }
);
const isSchoolSessionSnapshot = (
  value: unknown,
): value is SchoolSessionSnapshot => {
  if (!value || typeof value !== "object") return false;
  const { revision, profiles } = value as Partial<SchoolSessionSnapshot>;
  return (
    Number.isSafeInteger(revision) &&
    (revision as number) >= 0 &&
    Boolean(profiles) &&
    Array.isArray(profiles?.science?.presentation?.recovery) &&
    Array.isArray(profiles?.tokyoTech?.presentation?.recovery) &&
    SCHOOL_SESSION_STATES.includes(profiles?.science?.state) &&
    SCHOOL_SESSION_STATES.includes(profiles?.tokyoTech?.state)
  );
};
export interface SchoolSessionBridge {
  schoolCredentialStatus(): Promise<SchoolCredentialStatus>;
  /** Signs in with the entered credentials; saves them only when the school
   * accepts them. */
  connectSchool(
    profile: SchoolAuthProfile,
    credentials: SchoolCredentials,
  ): Promise<SchoolConnectOutcome>;
  deleteSchoolCredentials(profile: SchoolAuthProfile): Promise<boolean>;
  /** Copies the saved Matrix to the system clipboard without exposing it to the WebView. */
  copySchoolMatrix(): Promise<boolean>;
  copySchoolOtp(setup?: string): Promise<boolean>;
  /** SVG QR code of the OTP setup, for another authenticator to scan. */
  schoolOtpQr(): Promise<string>;
  schoolOtpCode(): Promise<SchoolOtpCode | null>;
  copySchoolOtpCode(otp: string): Promise<void>;
  schoolSessionSnapshot(): Promise<SchoolSessionSnapshot>;
  /** One foreground sign-in with the saved credentials, read again from the
   * synced store; the credentials themselves do not change. */
  retrySchool(profile: SchoolAuthProfile): Promise<SchoolSessionSnapshot>;
  onSchoolSession(
    callback: (snapshot: SchoolSessionSnapshot) => void,
  ): Promise<() => void> | (() => void);
}

export function schoolSessionCommands(
  { result, invokeCommand, booleanCommand }: BridgeCommands,
  listen: ListenNative,
): SchoolSessionBridge {
  return {
    schoolCredentialStatus: () => result("school_credential_status"),
    connectSchool: (profile, credentials) =>
      result("connect_school", { profile, credentials }),
    deleteSchoolCredentials: (profile) =>
      booleanCommand("delete_school_credentials", { profile }),
    copySchoolMatrix: () => booleanCommand("copy_school_matrix"),
    copySchoolOtp: (setup) => booleanCommand("copy_school_otp", { setup }),
    schoolOtpCode: () => invokeCommand<SchoolOtpCode | null>("school_otp_code"),
    copySchoolOtpCode: (otp) =>
      invokeCommand<void>("copy_school_otp_code", { otp }),
    schoolOtpQr: async () =>
      requireString("school_otp_qr", await invokeCommand("school_otp_qr")),
    schoolSessionSnapshot: async () => {
      const snapshot = await invokeCommand("school_session_snapshot");
      if (!isSchoolSessionSnapshot(snapshot))
        throw new Error("school_session_snapshot returned an invalid result.");
      return snapshot;
    },
    retrySchool: async (profile) => {
      const snapshot = await invokeCommand("retry_school", { profile });
      if (!isSchoolSessionSnapshot(snapshot))
        throw new Error("retry_school returned an invalid result.");
      return snapshot;
    },
    onSchoolSession: async (callback) => {
      let disposed = false;
      const unlisten = await listen<unknown>("school-session", (payload) => {
        if (!disposed && isSchoolSessionSnapshot(payload)) callback(payload);
      });
      return () => {
        disposed = true;
        quietly(unlisten);
      };
    },
  };
}

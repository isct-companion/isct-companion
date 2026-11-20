export type SchoolAuthProfile = "science" | "tokyo-tech";
/** One account's school session, as Core's session read model names it. */
export type SchoolSessionState =
  /** No saved credentials. */
  | "not-configured"
  /** Nothing stands in the way: Core signs in when it needs to. */
  | "normal"
  | "signing-in"
  /** The school refused the saved credentials; Core does not sign in with
   * them on its own until they change or the person retries. */
  | "refused"
  /** The person signs in on the school page (email code, password change,
   * a page Core does not know). */
  | "needs-web"
  /** Network, timeout or server failure; Core tries again after `retryAt`. */
  | "unavailable";
export type SchoolSignInProblemKind =
  /** With `refused`. */
  | "invalid-credentials"
  | "invalid-otp"
  /** A Connect without an OTP setup reached the App OTP step. */
  | "otp-needed"
  | "invalid-matrix"
  /** With `needs-web`. */
  | "email-otp"
  | "password-change"
  | "unknown-page"
  /** With `unavailable`. */
  | "network"
  | "unanswered"
  | "anomaly"
  | "credential-store";
export type SignInRecovery =
  "retry" | "open-school-page" | "set-up-otp-on-portal" | "finish-sign-out";
export type SignInPresentation = {
  stateKey: string;
  reasonKey: string | null;
  recovery: SignInRecovery[];
};
/** Why the last sign-in did not finish. `message` is Core's English
 * diagnostic, for logs and tooltips; the shell shows text chosen by `kind`. */
export type SchoolSignInProblem = {
  kind: SchoolSignInProblemKind;
  message: string;
};
/** LMS (science) or 教務Web (tokyoTech) paused for maintenance. */
export type SchoolMaintenance = {
  /** The school's own notice, when it posted one. */
  notice: string | null;
  /** RFC 3339. */
  until: string;
};
export type SchoolProfileSession = {
  signOutPending: boolean;
  refusalUnsaved: boolean;
  state: SchoolSessionState;
  presentation: SignInPresentation;
  problem: SchoolSignInProblem | null;
  /** RFC 3339; only with `unavailable`. */
  retryAt: string | null;
  maintenance: SchoolMaintenance | null;
};
/** Both accounts' sessions; every event carries the whole snapshot, and the
 * higher `revision` wins. */
export type SchoolSessionSnapshot = {
  revision: number;
  profiles: {
    science: SchoolProfileSession;
    tokyoTech: SchoolProfileSession;
  };
};

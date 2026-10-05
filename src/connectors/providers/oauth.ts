import type { CloudProvider } from "../cloud";
import {
  AuthenticationError,
  AuthorizationRedirectError,
  ConfigurationError,
} from "./errors";
export interface OAuthStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}
export interface OAuthSession {
  connect(): Promise<void>;
  requireAccessToken(): Promise<string>;
  disconnect?(): Promise<void> | void;
}
type Options = {
  storage?: OAuthStorage;
  navigate?: (url: string) => void;
  fetch?: typeof globalThis.fetch;
};
const b64 = (x: Uint8Array) =>
  btoa(String.fromCharCode(...x))
    .replaceAll("+", "-")
    .replaceAll("/", "_")
    .replaceAll("=", "");
async function pkce() {
  const verifier = b64(crypto.getRandomValues(new Uint8Array(32))),
    digest = await crypto.subtle.digest(
      "SHA-256",
      new TextEncoder().encode(verifier),
    );
  return { verifier, challenge: b64(new Uint8Array(digest)) };
}
export class BrowserOAuthSession implements OAuthSession {
  private storage: OAuthStorage;
  private prefix: string;
  constructor(
    private provider: CloudProvider,
    private clientId: string,
    private redirectUri: string,
    private options: Options = {},
  ) {
    if (!/^https?:\/\//.test(redirectUri))
      throw new ConfigurationError(
        "OAuth redirect URI must be an absolute registered HTTP(S) URL.",
      );
    const fallback =
      typeof sessionStorage === "undefined" ? undefined : sessionStorage;
    if (!options.storage && !fallback)
      throw new ConfigurationError(
        "Session storage is required for OAuth state.",
      );
    this.storage = options.storage ?? fallback!;
    this.prefix = `isct.oauth.${provider}.`;
  }
  async connect() {
    const current = new URL(globalThis.location?.href ?? this.redirectUri),
      error = current.searchParams.get("error");
    if (error)
      throw new AuthenticationError(`OAuth authorization failed: ${error}`);
    const code = current.searchParams.get("code");
    if (code) {
      await this.exchange(code, current.searchParams.get("state"));
      return;
    }
    const { verifier, challenge } = await pkce(),
      state = b64(crypto.getRandomValues(new Uint8Array(24)));
    this.storage.setItem(this.prefix + "verifier", verifier);
    this.storage.setItem(this.prefix + "state", state);
    const auth = new URL(
      this.provider === "google-drive"
        ? "https://accounts.google.com/o/oauth2/v2/auth"
        : "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
    );
    auth.search = new URLSearchParams({
      client_id: this.clientId,
      redirect_uri: this.redirectUri,
      response_type: "code",
      scope:
        this.provider === "google-drive"
          ? "openid https://www.googleapis.com/auth/drive.appdata"
          : "openid Files.ReadWrite.AppFolder",
      state,
      code_challenge: challenge,
      code_challenge_method: "S256",
      prompt: "select_account",
    }).toString();
    (this.options.navigate ?? ((url: string) => location.assign(url)))(
      auth.toString(),
    );
    throw new AuthorizationRedirectError("Redirecting to the cloud provider.");
  }
  async requireAccessToken() {
    const raw = this.storage.getItem(this.prefix + "token");
    if (!raw) throw new AuthenticationError("Cloud account is not connected.");
    const token = JSON.parse(raw) as { accessToken: string; expiresAt: number };
    if (token.expiresAt <= Date.now() + 30000) {
      this.storage.removeItem(this.prefix + "token");
      throw new AuthenticationError(
        "Cloud authorization expired. Reconnect the account.",
      );
    }
    return token.accessToken;
  }
  disconnect() {
    this.storage.removeItem(this.prefix + "token");
  }
  private async exchange(code: string, state: string | null) {
    const expected = this.storage.getItem(this.prefix + "state"),
      verifier = this.storage.getItem(this.prefix + "verifier");
    if (!state || state !== expected || !verifier)
      throw new AuthenticationError("OAuth callback state validation failed.");
    const endpoint =
      this.provider === "google-drive"
        ? "https://oauth2.googleapis.com/token"
        : "https://login.microsoftonline.com/common/oauth2/v2.0/token";
    const body = new URLSearchParams({
      client_id: this.clientId,
      redirect_uri: this.redirectUri,
      grant_type: "authorization_code",
      code,
      code_verifier: verifier,
    });
    const response = await (this.options.fetch ?? fetch)(endpoint, {
      method: "POST",
      headers: { "Content-Type": "application/x-www-form-urlencoded" },
      body,
    });
    if (!response.ok)
      throw new AuthenticationError(
        `OAuth token exchange failed (${response.status}). Check the registered public-client redirect URI.`,
      );
    const data = (await response.json()) as {
      access_token?: string;
      expires_in?: number;
    };
    if (!data.access_token)
      throw new AuthenticationError("OAuth provider returned no access token.");
    this.storage.setItem(
      this.prefix + "token",
      JSON.stringify({
        accessToken: data.access_token,
        expiresAt: Date.now() + (data.expires_in ?? 3600) * 1000,
      }),
    );
    this.storage.removeItem(this.prefix + "state");
    this.storage.removeItem(this.prefix + "verifier");
  }
}

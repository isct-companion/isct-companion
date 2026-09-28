import type { Workspace } from "../core/model";
import {
  AuthenticationError,
  AuthorizationRedirectError,
  CloudConflictError,
  CloudTransportError,
  ConfigurationError,
} from "./providers/errors";
import {
  BrowserOAuthSession,
  type OAuthSession,
  type OAuthStorage,
} from "./providers/oauth";
import { GoogleDriveBackupTransport } from "./providers/googleDrive";
import { OneDriveBackupTransport } from "./providers/oneDrive";
import type {
  BackupManifest,
  CloudBackupTransport,
} from "./providers/transport";

export {
  AuthenticationError,
  AuthorizationRedirectError,
  CloudConflictError,
  CloudTransportError,
  ConfigurationError,
};
export type CloudProvider = "google-drive" | "onedrive";
export type ConflictChoice = "keep-current" | "use-latest" | "resolve-manually";
export interface PayloadCipher {
  readonly keyId: string;
  encrypt(cleartext: Uint8Array): Promise<Uint8Array>;
  decrypt(ciphertext: Uint8Array): Promise<Uint8Array>;
}
export interface CloudAdapter {
  provider: CloudProvider;
  connect(): Promise<void>;
  pull(): Promise<Workspace>;
  push(data: Workspace): Promise<void>;
}
export interface CloudAdapterOptions {
  redirectUri?: string;
  cipher?: PayloadCipher;
  oauth?: OAuthSession;
  storage?: OAuthStorage;
  fetch?: typeof globalThis.fetch;
  now?: () => Date;
  navigate?: (url: string) => void;
  readOnly?: boolean;
  deviceId?: string;
}
const encode = new TextEncoder(),
  decode = new TextDecoder();

async function sha256(bytes: Uint8Array) {
  const hash = await crypto.subtle.digest(
    "SHA-256",
    new Uint8Array(bytes).buffer,
  );
  return [...new Uint8Array(hash)]
    .map((x) => x.toString(16).padStart(2, "0"))
    .join("");
}
function parseWorkspace(bytes: Uint8Array): Workspace {
  let value: unknown;
  try {
    value = JSON.parse(decode.decode(bytes));
  } catch {
    throw new CloudTransportError("Cloud backup is not valid JSON.");
  }
  if (
    !value ||
    typeof value !== "object" ||
    (value as Workspace).schemaVersion !== 2
  )
    throw new CloudTransportError("Cloud backup schema is unsupported.");
  return value as Workspace;
}

class VersionedBackupAdapter implements CloudAdapter {
  private observedVersion: number | null = null;
  constructor(
    readonly provider: CloudProvider,
    private oauth: OAuthSession,
    private transport: CloudBackupTransport,
    private cipher: PayloadCipher,
    private options: CloudAdapterOptions,
  ) {}
  async connect() {
    await this.oauth.connect();
  }
  async pull() {
    await this.oauth.requireAccessToken();
    const manifest = await this.transport.readManifest();
    if (!manifest)
      throw new CloudTransportError("No cloud backup exists yet.", 404, false);
    if (
      manifest.format !== "isct-backup-meta" ||
      manifest.schemaVersion !== 1 ||
      !Number.isSafeInteger(manifest.version) ||
      manifest.version < 1 ||
      !/^[a-f0-9]{64}$/.test(manifest.sha256) ||
      !Number.isSafeInteger(manifest.size) ||
      manifest.size < 1 ||
      !manifest.createdBy ||
      Number.isNaN(Date.parse(manifest.createdAt))
    )
      throw new CloudTransportError(
        "Cloud backup metadata is invalid or unsupported. Keeping cached data.",
        409,
        false,
      );
    if (manifest.keyId !== this.cipher.keyId)
      throw new CloudTransportError(
        "Cloud backup uses a different recovery key. Keeping cached data.",
        409,
        false,
      );
    const encrypted = await this.transport.readBackup();
    if (!encrypted)
      throw new CloudTransportError(
        "Cloud backup is temporarily incomplete. Keeping cached data; retry sync.",
        409,
        true,
      );
    if (encrypted.byteLength !== manifest.size)
      throw new CloudTransportError(
        "Cloud backup size does not match its metadata. Keeping cached data; retry sync.",
        409,
        true,
      );
    if ((await sha256(encrypted)) !== manifest.sha256)
      throw new CloudTransportError(
        "Cloud backup and metadata do not match. Keeping cached data; retry sync.",
        409,
        true,
      );
    const workspace = parseWorkspace(await this.cipher.decrypt(encrypted));
    this.observedVersion = manifest.version;
    return workspace;
  }
  async push(workspace: Workspace) {
    if (this.options.readOnly)
      throw new ConfigurationError(
        "This browser preview is read-only. Create backups from the desktop app.",
      );
    await this.oauth.requireAccessToken();
    const remote = await this.transport.readManifest();
    if (remote && this.observedVersion === null)
      throw new CloudConflictError(
        remote,
        "Cloud already contains a backup. Pull it before choosing Keep current, Use latest, or Resolve manually.",
      );
    if (remote && remote.version !== this.observedVersion)
      throw new CloudConflictError(
        remote,
        "Cloud backup changed since it was read. Choose Keep current, Use latest, or Resolve manually.",
      );
    const encrypted = await this.cipher.encrypt(
      encode.encode(JSON.stringify(workspace)),
    );
    const manifest: BackupManifest = {
      format: "isct-backup-meta",
      schemaVersion: 1,
      version: (remote?.version ?? 0) + 1,
      sha256: await sha256(encrypted),
      size: encrypted.byteLength,
      createdAt: (this.options.now ?? (() => new Date()))().toISOString(),
      createdBy: this.options.deviceId ?? workspace.deviceId,
      keyId: this.cipher.keyId,
    };
    await this.transport.writeBackup(encrypted);
    await this.transport.writeManifest(manifest);
    this.observedVersion = manifest.version;
  }
}

export function createCloudAdapter(
  provider: CloudProvider,
  clientId: string,
  options: CloudAdapterOptions = {},
): CloudAdapter {
  if (!clientId.trim())
    throw new ConfigurationError(
      `${provider === "google-drive" ? "Google" : "Microsoft"} OAuth client ID is required.`,
    );
  const fail = (message: string): CloudAdapter => {
    const reject = async () => {
      throw new ConfigurationError(message);
    };
    return { provider, connect: reject, pull: reject, push: reject };
  };
  if (!options.redirectUri && !options.oauth)
    return fail(
      "OAuth redirect URI must be configured for this installation before cloud data can be accessed.",
    );
  if (!options.cipher)
    return fail(
      "Encrypted cloud sync requires a configured device-key cipher.",
    );
  const oauth =
    options.oauth ??
    new BrowserOAuthSession(provider, clientId, options.redirectUri!, {
      storage: options.storage,
      navigate: options.navigate,
      fetch: options.fetch,
    });
  const request = async (
    url: string,
    init: RequestInit = {},
    allowNotFound = false,
  ) => {
    const token = await oauth.requireAccessToken();
    const response = await (options.fetch ?? fetch)(url, {
      ...init,
      headers: { ...init.headers, Authorization: `Bearer ${token}` },
    });
    if (allowNotFound && response.status === 404) return response;
    if (response.status === 401)
      throw new AuthenticationError(
        "Cloud authorization expired. Reconnect the account.",
      );
    if (!response.ok)
      throw new CloudTransportError(
        `Cloud request failed (${response.status}).`,
        response.status,
        response.status === 429 || response.status >= 500,
      );
    return response;
  };
  const transport =
    provider === "google-drive"
      ? new GoogleDriveBackupTransport(request)
      : new OneDriveBackupTransport(request);
  return new VersionedBackupAdapter(
    provider,
    oauth,
    transport,
    options.cipher,
    options,
  );
}
export type {
  OAuthSession,
  OAuthStorage,
  BackupManifest,
  CloudBackupTransport,
};

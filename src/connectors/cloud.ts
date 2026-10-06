import {
  validateExportDocument,
  type ExportDocument,
} from "../storage/repository";
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
import type { CloudBackupTransport } from "./providers/transport";

export {
  AuthenticationError,
  AuthorizationRedirectError,
  CloudConflictError,
  CloudTransportError,
  ConfigurationError,
};
export type CloudProvider = "google-drive" | "onedrive";
export const GOOGLE_DRIVE_CLIENT_ID =
  import.meta.env.VITE_GOOGLE_DRIVE_CLIENT_ID?.trim() ?? "";

export function resolveCloudClientId(
  provider: CloudProvider,
  override: string,
) {
  return (
    override.trim() ||
    (provider === "google-drive" ? GOOGLE_DRIVE_CLIENT_ID : "")
  );
}

export type ConflictChoice = "keep-current" | "use-latest" | "resolve-manually";
export interface CloudAdapter {
  provider: CloudProvider;
  connect(): Promise<void>;
  pull(): Promise<string>;
  push(document: string): Promise<void>;
}
export interface CloudAdapterOptions {
  redirectUri?: string;
  oauth?: OAuthSession;
  transport?: CloudBackupTransport;
  storage?: OAuthStorage;
  fetch?: typeof globalThis.fetch;
  now?: () => Date;
  navigate?: (url: string) => void;
  readOnly?: boolean;
  deviceId?: string;
}
const encode = new TextEncoder(),
  decode = new TextDecoder();

type CloudFile = {
  format: "IsctCompanion";
  formatVersion: 1;
  revision: number;
  updatedAt: string;
  updatedBy: string;
  document: ExportDocument;
};

function validExport(value: unknown): value is ExportDocument {
  try {
    validateExportDocument(value);
    return true;
  } catch {
    return false;
  }
}

function parseCloudFile(bytes: Uint8Array): CloudFile {
  let value: unknown;
  try {
    value = JSON.parse(decode.decode(bytes));
  } catch {
    throw new CloudTransportError("Cloud workspace is not valid JSON.");
  }
  const file = value as Partial<CloudFile>;
  if (
    !value ||
    typeof value !== "object" ||
    file.format !== "IsctCompanion" ||
    file.formatVersion !== 1 ||
    !Number.isSafeInteger(file.revision) ||
    (file.revision ?? 0) < 1 ||
    typeof file.updatedAt !== "string" ||
    Number.isNaN(Date.parse(file.updatedAt)) ||
    typeof file.updatedBy !== "string" ||
    !file.updatedBy ||
    !validExport(file.document)
  )
    throw new CloudTransportError(
      "Cloud workspace format is invalid or unsupported. Keeping local data.",
      409,
      false,
    );
  return file as CloudFile;
}

class PlaintextCloudAdapter implements CloudAdapter {
  private observedRevision: number | null = null;
  constructor(
    readonly provider: CloudProvider,
    private oauth: OAuthSession,
    private transport: CloudBackupTransport,
    private options: CloudAdapterOptions,
  ) {}
  async connect() {
    await this.oauth.connect();
  }
  async pull() {
    await this.oauth.requireAccessToken();
    const bytes = await this.transport.read();
    if (!bytes)
      throw new CloudTransportError(
        "No cloud workspace exists yet.",
        404,
        false,
      );
    const file = parseCloudFile(bytes);
    this.observedRevision = file.revision;
    return JSON.stringify(file.document);
  }
  async push(document: string) {
    if (this.options.readOnly)
      throw new ConfigurationError(
        "This browser preview is read-only. Create backups from the desktop app.",
      );
    await this.oauth.requireAccessToken();
    const remoteBytes = await this.transport.read();
    const remote = remoteBytes ? parseCloudFile(remoteBytes) : null;
    if (remote && this.observedRevision === null)
      throw new CloudConflictError(
        {
          version: remote.revision,
          createdAt: remote.updatedAt,
          createdBy: remote.updatedBy,
        },
        "Cloud already contains a workspace. Pull it before choosing Keep current, Use latest, or Resolve manually.",
      );
    if (remote && remote.revision !== this.observedRevision)
      throw new CloudConflictError(
        {
          version: remote.revision,
          createdAt: remote.updatedAt,
          createdBy: remote.updatedBy,
        },
        "Cloud workspace changed since it was read. Choose Keep current, Use latest, or Resolve manually.",
      );
    let data: ExportDocument;
    try {
      data = JSON.parse(document) as ExportDocument;
    } catch {
      throw new CloudTransportError("Export is not valid JSON.");
    }
    if (!validExport(data))
      throw new CloudTransportError("Export schema is unsupported.");
    const file: CloudFile = {
      format: "IsctCompanion",
      formatVersion: 1,
      revision: (remote?.revision ?? 0) + 1,
      updatedAt: (this.options.now ?? (() => new Date()))().toISOString(),
      updatedBy: this.options.deviceId ?? data.deviceId,
      document: data,
    };
    await this.transport.write(encode.encode(JSON.stringify(file, null, 2)));
    this.observedRevision = file.revision;
  }
}

export function createCloudAdapter(
  provider: CloudProvider,
  clientId: string,
  options: CloudAdapterOptions = {},
): CloudAdapter {
  const resolvedClientId = resolveCloudClientId(provider, clientId);
  if (!resolvedClientId)
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
  const oauth =
    options.oauth ??
    new BrowserOAuthSession(provider, resolvedClientId, options.redirectUri!, {
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
    options.transport ??
    (provider === "google-drive"
      ? new GoogleDriveBackupTransport(request)
      : new OneDriveBackupTransport(request));
  return new PlaintextCloudAdapter(provider, oauth, transport, options);
}
export type { OAuthSession, OAuthStorage, CloudBackupTransport };

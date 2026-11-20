import { subscribe, type BridgeCommands, type ListenNative } from "./helpers";

/** Automatic sync, per device; contract in slotledger approach/cloud-upload-queue. */
export type CloudSyncProvider = "icloud" | "google-drive";
export type CloudSyncVersion = {
  id: string;
  provider: CloudSyncProvider;
  savedAt: string;
  device: string;
  version?: string;
};
type CloudSyncTask = {
  id: string;
  direction: "upload" | "pull";
  version: string;
  createdAt: string;
  deliveries: {
    provider: CloudSyncProvider;
    status: "pending" | "running" | "completed";
    attempts: number;
    retryAt: number;
    error?: string;
  }[];
};
export type CloudSyncState = {
  syncing: boolean;
  automatic?: boolean;
  localVersion?: string;
  tasks?: CloudSyncTask[];
  providers: {
    provider: CloudSyncProvider;
    enabled: boolean;
    lastSyncedAt?: string;
    syncedVersion?: string;
    error?: string;
  }[];
  /** Earlier cloud copies, newest first. */
  versions: CloudSyncVersion[];
};

export interface CloudSyncBridge {
  cloudLifecycle?(kind: "foreground" | "background"): Promise<void>;
  cloudSyncState?(): Promise<CloudSyncState>;
  setCloudSyncProvider?(
    provider: CloudSyncProvider,
    enabled: boolean,
  ): Promise<CloudSyncState>;
  syncCloudNow?(direction: "upload" | "pull"): Promise<CloudSyncState>;
  setCloudSyncAutomatic?(automatic: boolean): Promise<CloudSyncState>;
  restoreCloudVersion?(id: string): Promise<CloudSyncState>;
  onCloudSyncState?(callback: (state: CloudSyncState) => void): () => void;
  cloudCredentialSyncStatus(): Promise<boolean>;
  cloudCredentialBackupStatus(): Promise<boolean>;
  /** Restores the Drive backup with `password` on a device without one;
   * otherwise uploads this device's logins under `password`, a new one. */
  enableCloudCredentialSync(password: string): Promise<boolean>;
  clearCloudCredentialSyncPassword(): Promise<boolean>;
}

export function cloudSyncCommands(
  { result, booleanCommand }: BridgeCommands,
  listen: ListenNative,
): CloudSyncBridge {
  return {
    cloudLifecycle: (kind) => result<void>("cloud_lifecycle", { kind }),
    cloudSyncState: () => result<CloudSyncState>("cloud_sync_state"),
    setCloudSyncProvider: (provider, enabled) =>
      result<CloudSyncState>("set_cloud_sync_provider", { provider, enabled }),
    syncCloudNow: (direction) =>
      result<CloudSyncState>("sync_cloud_now", { direction }),
    setCloudSyncAutomatic: (automatic) =>
      result<CloudSyncState>("set_cloud_sync_automatic", { automatic }),
    restoreCloudVersion: (id) =>
      result<CloudSyncState>("restore_cloud_version", { id }),
    onCloudSyncState: (callback) =>
      subscribe<CloudSyncState>(
        listen,
        "cloud-sync-state",
        (payload) =>
          payload && Array.isArray(payload.providers) ? payload : undefined,
        callback,
      ),
    cloudCredentialSyncStatus: () =>
      booleanCommand("cloud_credential_sync_status"),
    cloudCredentialBackupStatus: () =>
      booleanCommand("cloud_credential_backup_status"),
    enableCloudCredentialSync: (password) =>
      booleanCommand("enable_cloud_credential_sync", { password }),
    clearCloudCredentialSyncPassword: () =>
      booleanCommand("clear_cloud_credential_sync_password"),
  };
}

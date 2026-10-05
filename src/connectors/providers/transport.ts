export const BACKUP_NAME = "backup.bin",
  META_NAME = "_meta.json";
export interface BackupManifest {
  format: "isct-backup-meta";
  schemaVersion: 1;
  version: number;
  sha256: string;
  size: number;
  createdAt: string;
  createdBy: string;
  keyId: string;
}
export interface CloudBackupTransport {
  readManifest(): Promise<BackupManifest | null>;
  readBackup(): Promise<Uint8Array | null>;
  writeBackup(bytes: Uint8Array): Promise<void>;
  writeManifest(manifest: BackupManifest): Promise<void>;
}
export type AuthorizedRequest = (
  url: string,
  init?: RequestInit,
  allowNotFound?: boolean,
) => Promise<Response>;

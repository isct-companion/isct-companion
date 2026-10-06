export const CLOUD_FILE_NAME = "IsctCompanion.json";
export interface CloudBackupTransport {
  read(): Promise<Uint8Array | null>;
  write(bytes: Uint8Array): Promise<void>;
}
export type AuthorizedRequest = (
  url: string,
  init?: RequestInit,
  allowNotFound?: boolean,
) => Promise<Response>;

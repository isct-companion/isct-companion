import {
  BACKUP_NAME,
  META_NAME,
  type AuthorizedRequest,
  type BackupManifest,
  type CloudBackupTransport,
} from "./transport";
const graph = "https://graph.microsoft.com/v1.0";
export class OneDriveBackupTransport implements CloudBackupTransport {
  constructor(private request: AuthorizedRequest) {}
  private async read(name: string) {
    const response = await this.request(
      `${graph}/me/drive/special/approot:/${name}:/content`,
      {},
      true,
    );
    if (response.status === 404) return null;
    return new Uint8Array(await response.arrayBuffer());
  }
  private async write(name: string, bytes: Uint8Array, contentType: string) {
    await this.request(`${graph}/me/drive/special/approot:/${name}:/content`, {
      method: "PUT",
      headers: { "Content-Type": contentType },
      body: new Blob([new Uint8Array(bytes)]),
    });
  }
  async readManifest() {
    const bytes = await this.read(META_NAME);
    if (!bytes) return null;
    return JSON.parse(new TextDecoder().decode(bytes)) as BackupManifest;
  }
  readBackup() {
    return this.read(BACKUP_NAME);
  }
  writeBackup(bytes: Uint8Array) {
    return this.write(BACKUP_NAME, bytes, "application/octet-stream");
  }
  writeManifest(manifest: BackupManifest) {
    return this.write(
      META_NAME,
      new TextEncoder().encode(JSON.stringify(manifest)),
      "application/json",
    );
  }
}

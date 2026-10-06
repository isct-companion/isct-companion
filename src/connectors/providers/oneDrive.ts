import {
  CLOUD_FILE_NAME,
  type AuthorizedRequest,
  type CloudBackupTransport,
} from "./transport";
const graph = "https://graph.microsoft.com/v1.0";
export class OneDriveBackupTransport implements CloudBackupTransport {
  constructor(private request: AuthorizedRequest) {}
  private async readFile(name: string) {
    const response = await this.request(
      `${graph}/me/drive/special/approot:/${name}:/content`,
      {},
      true,
    );
    if (response.status === 404) return null;
    return new Uint8Array(await response.arrayBuffer());
  }
  private async writeFile(
    name: string,
    bytes: Uint8Array,
    contentType: string,
  ) {
    await this.request(`${graph}/me/drive/special/approot:/${name}:/content`, {
      method: "PUT",
      headers: { "Content-Type": contentType },
      body: new Blob([new Uint8Array(bytes)]),
    });
  }
  read() {
    return this.readFile(CLOUD_FILE_NAME);
  }
  write(bytes: Uint8Array) {
    return this.writeFile(CLOUD_FILE_NAME, bytes, "application/json");
  }
}

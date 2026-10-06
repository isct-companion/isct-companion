import {
  CLOUD_FILE_NAME,
  type AuthorizedRequest,
  type CloudBackupTransport,
} from "./transport";
type DriveFile = { id: string; name: string };
export class GoogleDriveBackupTransport implements CloudBackupTransport {
  constructor(private request: AuthorizedRequest) {}
  private async find(name: string) {
    const q = `name = '${name}' and trashed = false`;
    const response = await this.request(
      `https://www.googleapis.com/drive/v3/files?spaces=appDataFolder&q=${encodeURIComponent(q)}&fields=files(id,name)&pageSize=2`,
    );
    const data = (await response.json()) as { files?: DriveFile[] };
    return data.files?.[0] ?? null;
  }
  private async readFile(name: string) {
    const file = await this.find(name);
    if (!file) return null;
    return new Uint8Array(
      await (
        await this.request(
          `https://www.googleapis.com/drive/v3/files/${encodeURIComponent(file.id)}?alt=media`,
        )
      ).arrayBuffer(),
    );
  }
  private async writeFile(
    name: string,
    bytes: Uint8Array,
    contentType: string,
  ) {
    const file = await this.find(name);
    if (file) {
      await this.request(
        `https://www.googleapis.com/upload/drive/v3/files/${encodeURIComponent(file.id)}?uploadType=media`,
        {
          method: "PATCH",
          headers: { "Content-Type": contentType },
          body: new Blob([new Uint8Array(bytes)]),
        },
      );
      return;
    }
    const boundary = `isct_${crypto.randomUUID().replaceAll("-", "")}`,
      metadata = JSON.stringify({ name, parents: ["appDataFolder"] }),
      a = new TextEncoder().encode(
        `--${boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n${metadata}\r\n--${boundary}\r\nContent-Type: ${contentType}\r\n\r\n`,
      ),
      b = new TextEncoder().encode(`\r\n--${boundary}--`),
      body = new Uint8Array(a.length + bytes.length + b.length);
    body.set(a);
    body.set(bytes, a.length);
    body.set(b, a.length + bytes.length);
    await this.request(
      "https://www.googleapis.com/upload/drive/v3/files?uploadType=multipart",
      {
        method: "POST",
        headers: { "Content-Type": `multipart/related; boundary=${boundary}` },
        body: new Blob([body]),
      },
    );
  }
  read() {
    return this.readFile(CLOUD_FILE_NAME);
  }
  write(bytes: Uint8Array) {
    return this.writeFile(CLOUD_FILE_NAME, bytes, "application/json");
  }
}

import { extension } from "./file-types";

export const megabytes = (bytes: number) =>
  `${Math.max(0.1, bytes / 1024 / 1024).toFixed(bytes < 10 * 1024 * 1024 ? 1 : 0)} MB`;
/** A file's size in the unit that reads best. */
export const fileSize = (bytes: number) =>
  bytes < 1024 * 1024
    ? `${Math.max(1, Math.round(bytes / 1024))} KB`
    : megabytes(bytes);

/** A path inside a ZIP or a folder's file name, as a URL `file-types` reads. */
export const byName = (name: string) =>
  `https://file.invalid/${encodeURIComponent(name)}`;

/**
 * A listed file's name over its folder inside a ZIP, its type and size.
 * Names carry the extension; folder URLs may not.
 */
export function FileName({ name, detail }: { name: string; detail?: string }) {
  const slash = name.lastIndexOf("/");
  const ext = extension(byName(name));
  return (
    <span>
      <b>{name.slice(slash + 1)}</b>
      <small>
        {[slash > 0 ? name.slice(0, slash) : "", ext.toUpperCase(), detail]
          .filter(Boolean)
          .join(" · ")}
      </small>
    </span>
  );
}

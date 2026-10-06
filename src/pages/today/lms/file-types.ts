export type PreviewKind = "pdf" | "markdown" | "text" | "image" | "video" | "audio";
/** Visual family of an LMS item; drives the row icon and its tint. */
export type FileFamily =
  | PreviewKind
  | "doc" | "slides" | "sheet" | "archive" | "file"
  | "link" | "folder" | "forum" | "page" | "note";

const families: [FileFamily, RegExp][] = [
  ["pdf", /^pdf$/],
  ["markdown", /^(md|markdown)$/],
  ["text", /^(txt|tex|bib|js|jsx|ts|tsx|java|py|ipynb|r|m|c|cpp|cxx|cc|h|hpp|rs|go|rb|sh|sql|json|yaml|yml|toml|xml|css|html?|csv|tsv|log|v|sv|vhdl?|asm|s|hs|ml|scala|kt|swift)$/],
  ["image", /^(png|apng|jpe?g|jpe|gif|webp|avif|bmp|ico|svg|tiff?|heic|heif)$/],
  ["video", /^(mp4|m4v|mov|webm|ogv)$/],
  ["audio", /^(mp3|m4a|aac|wav|ogg|oga|flac|opus)$/],
  ["doc", /^(docx?|odt|rtf|pages)$/],
  ["slides", /^(pptx?|odp|key)$/],
  ["sheet", /^(xlsx?|xlsm|ods|numbers)$/],
  ["archive", /^(zip|7z|rar|tar|gz|tgz|bz2|xz)$/],
];
// ipynb/html are recognised as text-like for the icon, but are not useful as
// raw text previews; they download instead.
const noPreview = new Set(["ipynb", "html", "htm"]);

export function extension(url: string): string {
  let path: string;
  try {
    path = new URL(url).pathname;
  } catch {
    path = url;
  }
  try {
    path = decodeURIComponent(path);
  } catch { /* Keep the raw path when it is not valid percent-encoding. */ }
  return path.match(/\.([a-z0-9]{1,8})$/i)?.[1].toLowerCase() ?? "";
}

export function fileFamily(url: string, module?: string): FileFamily {
  const ext = extension(url);
  const family = families.find(([, pattern]) => pattern.test(ext))?.[0];
  if (family) return family;
  if (module === "folder") return "folder";
  if (module === "forum") return "forum";
  if (module === "url") return "link";
  if (module === "page" || module === "label") return "page";
  return "file";
}

/** In-app preview available for this URL, or undefined to download it. */
export function previewKind(url: string): PreviewKind | undefined {
  const ext = extension(url);
  if (noPreview.has(ext)) return undefined;
  const family = families.find(([, pattern]) => pattern.test(ext))?.[0];
  return family === "pdf" || family === "markdown" || family === "text" || family === "image" || family === "video" || family === "audio"
    ? family
    : undefined;
}

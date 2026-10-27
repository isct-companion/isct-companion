import type { FileFamily } from "./file-types";

const sheet =
  "M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z M14 3v5h5";
const paths: Record<
  | FileFamily
  | "chevron"
  | "plus"
  | "minus"
  | "close"
  | "search"
  | "refresh"
  | "back"
  | "download"
  | "external"
  | "trash"
  | "code",
  string
> = {
  pdf: `${sheet} M9 13h6 M9 17h4`,
  doc: `${sheet} M9 12h6 M9 15h6 M9 18h3`,
  page: `${sheet} M9 12h6 M9 15h6 M9 18h3`,
  slides: `${sheet} M8.5 12h7v5h-7z`,
  sheet: `${sheet} M8.5 12h7v6h-7z M8.5 15h7 M12 12v6`,
  archive: `${sheet} M11 4v2 M11 8v2 M11 12v2 M10 16h2v2h-2z`,
  text: `${sheet} m10 13-2 2 2 2 M14 13l2 2-2 2`,
  markdown: `${sheet} M8 18v-5l2 2 2-2v5 M15.5 13v5 M14 16.5l1.5 1.5 1.5-1.5`,
  file: sheet,
  image:
    "M5 4h14a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z M3.5 17l5-5 4 4 2.5-2.5 5 5 M15.5 8.5h.01",
  video:
    "M5 5h14a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2z M10 9.5v5l4.5-2.5z",
  audio:
    "M9 18V6l11-2v12 M9 18a3 3 0 1 1-6 0 3 3 0 0 1 6 0z M20 16a3 3 0 1 1-6 0 3 3 0 0 1 6 0z",
  link: "M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1 M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1",
  folder:
    "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z",
  forum:
    "M5 4h14a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2h-8l-5 4v-4H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z M8 9h8 M8 12h5",
  note: "M4 20h4L19.5 8.5a2.1 2.1 0 0 0-4-4L4 16z M13.5 6.5l4 4",
  chevron: "m9 6 6 6-6 6",
  plus: "M12 5v14 M5 12h14",
  minus: "M5 12h14",
  close: "M6 6l12 12 M18 6 6 18",
  search: "M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14z m9 2-4-4",
  refresh:
    "M20 11a8 8 0 0 0-14.9-3 M4 4v4h4 M4 13a8 8 0 0 0 14.9 3 M20 20v-4h-4",
  back: "M19 12H5 m6-6-6 6 6 6",
  download: "M12 4v11 m-5-5 5 5 5-5 M5 20h14",
  trash:
    "M4 7h16 M10 11v6 M14 11v6 M6 7l1 12a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-12 M9 7V4h6v3",
  code: "m8 8-4 4 4 4 M16 8l4 4-4 4 M13.5 5l-3 14",
  external:
    "M14 4h6v6 M20 4l-9 9 M18 14v4a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4",
};
export type IconName = keyof typeof paths;

export function LmsIcon({
  name,
  size = 16,
  className,
}: {
  name: IconName;
  size?: number;
  className?: string;
}) {
  return (
    <svg
      className={className}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.75}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d={paths[name]} />
    </svg>
  );
}

/** Tinted tile for an outline row; `data-family` selects the tint in CSS. */
export function FileTile({ family }: { family: FileFamily }) {
  return (
    <span className="lmsFileTile" data-family={family} aria-hidden="true">
      <LmsIcon name={family} size={18} />
    </span>
  );
}

import {
  ArrowLeft,
  ChevronRight,
  Code,
  Copy,
  Download,
  ExternalLink,
  File,
  FileArchive,
  FileCode,
  FileSpreadsheet,
  FileText,
  FileType,
  Folder,
  ImageIcon,
  Link,
  MessagesSquare,
  Minus,
  Music,
  NotebookPen,
  Plus,
  Presentation,
  RefreshCw,
  Search,
  Share,
  Trash2,
  Video,
  X,
  type LucideIcon,
} from "lucide-react";
import type { FileFamily } from "./file-types";

/** LMS glyphs, all from lucide (slotledger approach/icons-per-platform). */
const glyphs: Record<
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
  | "copy"
  | "share"
  | "trash"
  | "code",
  LucideIcon
> = {
  pdf: FileText,
  doc: FileText,
  page: FileText,
  slides: Presentation,
  sheet: FileSpreadsheet,
  archive: FileArchive,
  text: FileCode,
  markdown: FileType,
  file: File,
  image: ImageIcon,
  video: Video,
  audio: Music,
  link: Link,
  folder: Folder,
  forum: MessagesSquare,
  note: NotebookPen,
  chevron: ChevronRight,
  plus: Plus,
  minus: Minus,
  close: X,
  search: Search,
  refresh: RefreshCw,
  back: ArrowLeft,
  download: Download,
  trash: Trash2,
  code: Code,
  external: ExternalLink,
  copy: Copy,
  share: Share,
};
export type IconName = keyof typeof glyphs;

export function LmsIcon({
  name,
  size = 16,
  className,
}: {
  name: IconName;
  size?: number;
  className?: string;
}) {
  const Glyph = glyphs[name];
  return (
    <Glyph
      className={className}
      size={size}
      strokeWidth={1.75}
      aria-hidden="true"
      focusable="false"
    />
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

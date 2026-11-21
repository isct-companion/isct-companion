import type { ReactNode } from "react";
import { ArrowLeft, ArrowRight } from "lucide-react";
import { useNavigate } from "react-router";
import { UiButton } from "./ui";
import { t } from "../i18n";

/** The history position react-router keeps in `history.state.idx`. */
const historyIndex = () =>
  (window.history.state as { idx?: number } | null)?.idx ?? 0;

/** Back and Forward through the pages the person visited, as in Codex and
 * Safari: the first buttons after the traffic lights. */
export function TitlebarNav({ sidebar }: { sidebar?: ReactNode }) {
  const navigate = useNavigate();
  return (
    <div className="titlebarNav">
      <UiButton
        type="button"
        className="iconButton"
        aria-label={t("common.back")}
        title={t("common.back")}
        disabled={historyIndex() === 0}
        onClick={() => void navigate(-1)}
      >
        <ArrowLeft />
      </UiButton>
      <UiButton
        type="button"
        className="iconButton"
        aria-label={t("common.forward")}
        title={t("common.forward")}
        onClick={() => void navigate(1)}
      >
        <ArrowRight />
      </UiButton>
      {sidebar}
    </div>
  );
}

/**
 * One column's header in the window's top row (`--titlebar`): its title
 * and its own controls, where a Mac app keeps its toolbar. The first column
 * after the rail also carries Back, Forward and the sidebar button
 * (`nav`). Empty space in the row moves the window.
 */
export function TitleRow({
  nav,
  sidebar,
  className,
  children,
}: {
  nav?: boolean;
  sidebar?: ReactNode;
  className?: string;
  children?: ReactNode;
}) {
  return (
    <header
      className={className ? `titleRow ${className}` : "titleRow"}
      data-tauri-drag-region
    >
      {nav && <TitlebarNav sidebar={sidebar} />}
      {children}
    </header>
  );
}

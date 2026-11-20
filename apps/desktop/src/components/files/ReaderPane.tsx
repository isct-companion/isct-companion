import { X } from "lucide-react";
import { createContext, useContext, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { t } from "../../i18n";
import "./reader.css";

const ToolsHost = createContext<HTMLElement | null>(null);

/** Document tools share one fixed panel; the reader below owns scrolling. */
export function ReaderPane({
  title,
  close,
  children,
}: {
  title: string;
  close: () => void;
  children: ReactNode;
}) {
  const [host, setHost] = useState<HTMLElement | null>(null);
  return (
    <section
      className="workspacePane lmsViewer"
      aria-label={t("lms.openedContent")}
    >
      <header className="workspaceToolPanel lmsReaderTools">
        <span className="lmsReaderTitle" title={title}>
          {title}
        </span>
        <div className="lmsReaderToolSlot" ref={setHost} />
        <button
          type="button"
          className="lmsViewerClose"
          aria-label={`${t("common.close")} ${title}`}
          title={t("common.close")}
          onClick={close}
        >
          <X size={14} strokeWidth={1.75} aria-hidden="true" focusable="false" />
        </button>
      </header>
      <ToolsHost.Provider value={host}>
        <div className="lmsViewerBody">{children}</div>
      </ToolsHost.Provider>
    </section>
  );
}

/** Standalone previews keep their own tools; embedded readers use the panel slot. */
export function ReaderTools({ children }: { children: ReactNode }) {
  const host = useContext(ToolsHost);
  return host ? createPortal(children, host) : children;
}

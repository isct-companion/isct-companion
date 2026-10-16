import { createContext, useContext, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { t } from "../../i18n";
import { LmsIcon } from "./LmsIcon";

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
      aria-label={t("Opened content")}
    >
      <header className="workspaceToolPanel lmsReaderTools">
        <span className="lmsReaderTitle" title={title}>
          {title}
        </span>
        <div className="lmsReaderToolSlot" ref={setHost} />
        <button
          type="button"
          className="lmsViewerClose"
          aria-label={`${t("Close")} ${title}`}
          title={t("Close")}
          onClick={close}
        >
          <LmsIcon name="close" size={14} />
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

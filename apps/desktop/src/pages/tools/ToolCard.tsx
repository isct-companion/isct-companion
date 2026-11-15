import type { ReactNode } from "react";
import { t, type MessageKey } from "../../i18n";

/** One tool: what it is, how it stands now and what can be done about it. */
export function ToolCard({
  icon,
  title,
  info,
  badge,
  status,
  note,
  actions,
  children,
}: {
  icon: ReactNode;
  title: MessageKey;
  /** Beside the title: what a curious reader can look up, on hover. */
  info?: ReactNode;
  /** At the header's end: the state in a word. */
  badge?: ReactNode;
  status?: ReactNode;
  note?: MessageKey;
  actions: ReactNode;
  children?: ReactNode;
}) {
  return (
    <article className="toolCard">
      <header>
        {icon}
        <h3>{t(title)}</h3>
        {info}
        {badge && <span className="toolCardBadge">{badge}</span>}
      </header>
      {status && <div className="toolCardStatus">{status}</div>}
      {note && <p className="toolCardNote">{t(note)}</p>}
      {children}
      <div className="toolCardActions">{actions}</div>
    </article>
  );
}

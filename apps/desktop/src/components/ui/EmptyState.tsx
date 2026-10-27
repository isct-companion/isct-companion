import type { LucideIcon } from "lucide-react";
import { CircleAlert, Inbox } from "lucide-react";
import type { ReactNode } from "react";
import { cx } from "./layout";
import { t } from "../../i18n";

/**
 * The one empty state (slotledger approach/empty-state): a muted icon, one
 * short line and at most one action, centred. `compact` sits inside a list
 * or a narrow pane and keeps the icon small.
 */
export function EmptyState({
  icon: Icon = Inbox,
  action,
  compact,
  className,
  role,
  children,
}: {
  icon?: LucideIcon;
  action?: ReactNode;
  compact?: boolean;
  className?: string;
  role?: "status" | "alert";
  children: ReactNode;
}) {
  return (
    <div
      className={cx("uiEmpty", className)}
      data-compact={compact || undefined}
      role={role}
    >
      <Icon aria-hidden size={compact ? 18 : 28} strokeWidth={1.75} />
      <p>{children}</p>
      {action}
    </div>
  );
}

/**
 * An error that makes a page or panel unusable (slotledger
 * approach/feedback-banner-toast): a danger-soft banner at its top with one
 * line and one "Retry". Transient results go to the toast instead.
 */
export function ErrorBanner({
  onRetry,
  className,
  title,
  children,
}: {
  onRetry?: () => void;
  className?: string;
  /** The underlying message, for hover; the line itself stays short. */
  title?: string;
  children: ReactNode;
}) {
  return (
    <div className={cx("uiBanner", className)} role="alert" title={title}>
      <CircleAlert aria-hidden size={16} strokeWidth={1.75} />
      <p>{children}</p>
      {onRetry && (
        <button type="button" className="uiBannerAction" onClick={onRetry}>
          {t("Retry")}
        </button>
      )}
    </div>
  );
}

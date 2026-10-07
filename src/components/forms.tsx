import { t } from "../i18n";
import { Badge, SegmentedControl, Switch } from "@mantine/core";
import { useId } from "react";
import {
  resolveCapabilities,
  type CapabilityId,
} from "../platform/capabilities";

/** Page title and one plain-language sentence about what the page is for. */
export function SettingsHeader({
  title,
  description,
  actions,
}: {
  title: string;
  description?: string;
  actions?: React.ReactNode;
}) {
  return (
    <header className="settingsHeader">
      <div>
        <h2>{t(title)}</h2>
        {description && <p>{t(description)}</p>}
      </div>
      {actions && <div className="actions">{actions}</div>}
    </header>
  );
}

/** A titled block of rows, like a grouped list in system settings. */
export function SettingsGroup({
  title,
  footer,
  children,
  tone,
}: {
  title?: string;
  footer?: React.ReactNode;
  children: React.ReactNode;
  tone?: "danger";
}) {
  return (
    <section className={`settingsGroup${tone ? ` ${tone}` : ""}`}>
      {title && <h3>{t(title)}</h3>}
      {children}
      {footer && <p className="groupFooter">{footer}</p>}
    </section>
  );
}

/** Rarely needed options stay folded until asked for. */
export function MoreOptions({
  label = "More options",
  children,
}: {
  label?: string;
  children: React.ReactNode;
}) {
  return (
    <details className="moreOptions">
      <summary>{t(label)}</summary>
      {children}
    </details>
  );
}

/**
 * One setting. `id` makes it reachable from settings search. A row with a
 * single control is a label, so clicking its title operates the control;
 * rows holding several controls (`group`) must not be.
 */
export function Row({
  id,
  title,
  hint,
  children,
  group,
}: {
  id?: string;
  title: string;
  hint?: React.ReactNode;
  children: React.ReactNode;
  group?: boolean;
}) {
  const labelId = useId();
  const text = (
    <span>
      <b id={labelId}>{t(title)}</b>
      {hint && <small>{typeof hint === "string" ? t(hint) : hint}</small>}
    </span>
  );
  return group ? (
    <div
      className="row"
      role="group"
      aria-labelledby={labelId}
      data-setting={id}
    >
      {text}
      {children}
    </div>
  ) : (
    <label className="row" data-setting={id}>
      {text}
      {children}
    </label>
  );
}

export function Toggle({
  checked,
  onChange,
  disabled,
}: {
  checked: boolean;
  onChange: (value: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <Switch
      className="settingSwitch"
      disabled={disabled}
      checked={checked}
      onChange={(event) => onChange(event.currentTarget.checked)}
    />
  );
}

/** A short choice list shown as one control; use inside `Row group`.
 * Labels are shown as given so language names can stay in their own script. */
export function Choice<T extends string>({
  value,
  options,
  onChange,
  disabled,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
  disabled?: boolean;
}) {
  return (
    <SegmentedControl
      className="settingChoice"
      size="sm"
      disabled={disabled}
      value={value}
      data={options}
      onChange={(next) => onChange(next as T)}
    />
  );
}

/** Compact state line: a colored dot, a sentence and an optional action. */
export function Status({
  id,
  tone,
  children,
  action,
}: {
  id?: string;
  tone: "ok" | "idle" | "warn" | "error";
  children: React.ReactNode;
  action?: React.ReactNode;
}) {
  return (
    <div className={`settingsStatus ${tone}`} role="status" data-setting={id}>
      <i aria-hidden />
      <span>{children}</span>
      {action}
    </div>
  );
}

export function Capability({
  name,
  value,
  children,
}: {
  name: string;
  value: ReturnType<typeof resolveCapabilities>[CapabilityId];
  children?: React.ReactNode;
}) {
  return (
    <div className="row" title={t(value.reason)}>
      <span>
        <b>{t(name)}</b>
        <small>{t(value.reason)}</small>
      </span>
      <span className="capState">
        {children}
        <Badge
          variant="light"
          color={
            value.support !== "supported"
              ? "gray"
              : value.readiness === "ready"
                ? "green"
                : value.readiness === "temporarily-unavailable"
                  ? "red"
                  : "yellow"
          }
        >
          {t(
            value.support === "supported"
              ? value.readiness.replaceAll("-", " ")
              : value.support,
          )}
        </Badge>
      </span>
    </div>
  );
}

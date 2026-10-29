import { t, type MessageKey } from "../i18n";
import { SegmentedControl, Switch } from "./ui";
import { useId } from "react";

/** Page title and one plain-language sentence about what the page is for. */
export function SettingsHeader({
  title,
  description,
  actions,
}: {
  title: MessageKey;
  description?: MessageKey;
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
  title?: MessageKey;
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
  label = "forms.moreOptions",
  children,
}: {
  label?: MessageKey;
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
  titleText,
  hint,
  children,
  group,
}: {
  id?: string;
  /** A catalog key; `titleText` instead holds text that is already final
   * (a date, a brand name). */
  title?: MessageKey;
  titleText?: string;
  hint?: React.ReactNode;
  children: React.ReactNode;
  group?: boolean;
}) {
  const labelId = useId();
  const text = (
    <span>
      <b id={labelId}>{title ? t(title) : titleText}</b>
      {hint && <small>{hint}</small>}
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
      onCheckedChange={onChange}
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

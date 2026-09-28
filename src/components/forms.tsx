import { t } from "../i18n";
import { Badge, Switch } from "@mantine/core";
import {
  resolveCapabilities,
  type CapabilityId,
} from "../platform/capabilities";
export function Row({
  title,
  hint,
  children,
}: {
  title: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <label className="row">
      <span>
        <b>{t(title)}</b>
        {hint && <small>{t(hint)}</small>}
      </span>
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

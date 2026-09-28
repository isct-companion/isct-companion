import { useId, useState } from "react";
import { UiButton, UiInput } from "./mantine";
import { t } from "../i18n";
/** Wrapped options keep long academic department labels readable and keyboard-selectable. */
export function SearchSelect({
  label,
  value,
  options,
  onChange,
  disabled = false,
  placeholder,
}: {
  label: string;
  value: string;
  options: { value: string; label: string }[];
  onChange: (value: string) => void;
  disabled?: boolean;
  placeholder: string;
}) {
  const id = useId();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const selected = options.find((o) => o.value === value);
  const visible = options.filter((o) =>
    o.label.toLocaleLowerCase().includes(query.toLocaleLowerCase()),
  );
  function choose(next: string) {
    onChange(next);
    setQuery("");
    setOpen(false);
  }
  return (
    <div className="searchSelect">
      <label htmlFor={id}>{label}</label>
      <UiInput
        id={id}
        role="combobox"
        aria-expanded={open}
        aria-controls={`${id}-options`}
        aria-autocomplete="list"
        aria-activedescendant={
          open && visible[active] ? `${id}-${active}` : undefined
        }
        autoComplete="off"
        disabled={disabled}
        value={open ? query : (selected?.label ?? "")}
        placeholder={placeholder}
        onFocus={() => {
          setOpen(true);
          setQuery("");
          setActive(0);
        }}
        onChange={(e) => {
          setQuery(e.target.value);
          setActive(0);
          setOpen(true);
        }}
        onBlur={() => setOpen(false)}
        onKeyDown={(e) => {
          if (e.nativeEvent.isComposing) return;
          if (e.key === "ArrowDown") {
            e.preventDefault();
            setOpen(true);
            setActive((i) => Math.min(i + 1, visible.length - 1));
          }
          if (e.key === "ArrowUp") {
            e.preventDefault();
            setActive((i) => Math.max(0, i - 1));
          }
          if (e.key === "Enter" && open) {
            e.preventDefault();
            if (visible[active]) choose(visible[active].value);
          }
          if (e.key === "Escape" && open) {
            e.preventDefault();
            e.stopPropagation();
            setOpen(false);
          }
        }}
      />
      {selected && !open && (
        <small className="selectedDepartment">{selected.label}</small>
      )}
      {open && (
        <div
          id={`${id}-options`}
          role="listbox"
          className="searchSelectOptions"
        >
          <UiButton
            role="option"
            aria-selected={!value}
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => choose("")}
          >
            {placeholder}
          </UiButton>
          {visible.map((o, i) => (
            <UiButton
              type="button"
              role="option"
              id={`${id}-${i}`}
              aria-selected={o.value === value}
              className={i === active ? "highlighted" : ""}
              key={o.value}
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => choose(o.value)}
            >
              {o.label}
            </UiButton>
          ))}
          {!visible.length && <p>{t("No matching departments")}</p>}
        </div>
      )}
    </div>
  );
}

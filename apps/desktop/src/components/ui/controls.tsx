import {
  Check as CheckGlyph,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  ChevronUp,
} from "lucide-react";
import {
  Checkbox as Check,
  RadioGroup,
  Switch as SwitchPrimitive,
  ToggleGroup,
  Progress as ProgressPrimitive,
  Select as SelectPrimitive,
  Popover as PopoverPrimitive,
} from "radix-ui";
import { Command } from "cmdk";
import {
  forwardRef,
  useId,
  useRef,
  useState,
  useEffect,
  type ButtonHTMLAttributes,
  type InputHTMLAttributes,
  type ReactNode,
  type RefObject,
} from "react";
import { usePageLayer } from "../../app/KeptPage";
import { t } from "../../i18n";
import { cx, type Size } from "./layout";

type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  size?: Size | "compact-xs" | "compact-sm";
  variant?: "filled" | "default" | "subtle" | "light";
  color?: string;
  loading?: boolean;
  leftSection?: ReactNode;
  px?: number;
};
export const Button = Object.assign(
  forwardRef<HTMLButtonElement, ButtonProps>(function Button(
    {
      size = "xs",
      variant = "filled",
      color,
      loading,
      disabled,
      leftSection,
      px,
      children,
      className,
      style,
      type = "button",
      ...props
    },
    ref,
  ) {
    return (
      <button
        {...props}
        ref={ref}
        type={type}
        disabled={disabled || loading}
        aria-busy={loading || undefined}
        className={cx("uiButton", className)}
        data-size={size}
        data-variant={variant}
        data-color={color}
        style={{ paddingInline: px, ...style }}
      >
        {loading ? (
          <i className="schoolStatusSpinner" aria-hidden />
        ) : (
          leftSection
        )}
        {children}
      </button>
    );
  }),
  {
    Group: ({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) => (
      <div {...props} className={cx("uiButtonGroup", className)} />
    ),
  },
);
export const UiButton = forwardRef<
  HTMLButtonElement,
  ButtonHTMLAttributes<HTMLButtonElement>
>(function UiButton({ className = "", ...props }, ref) {
  const classes = className.split(/\s+/);
  if (!classes.some((c) => ["button", "textButton", "iconButton"].includes(c)))
    return (
      <button
        type="button"
        {...props}
        ref={ref}
        className={cx("uiUnstyledButton", className)}
      />
    );
  return (
    <Button
      {...props}
      ref={ref}
      className={className}
      variant={
        classes.includes("primary")
          ? "filled"
          : classes.some((c) =>
                ["textButton", "iconButton", "quiet", "dangerText"].includes(c),
              )
            ? "subtle"
            : "default"
      }
      color={
        classes.some((c) => ["danger", "dangerText"].includes(c))
          ? "red"
          : classes.includes("iconButton")
            ? "gray"
            : undefined
      }
    />
  );
});
export const CloseButton = ({ size, ...props }: ButtonProps) => (
  <Button
    {...props}
    size={size}
    variant="subtle"
    className={cx("uiClose", props.className)}
    aria-label={props["aria-label"] ?? t("common.close")}
  >
    ×
  </Button>
);

type FieldProps = {
  label?: ReactNode;
  description?: ReactNode;
  error?: ReactNode;
  id?: string;
  className?: string;
  children?: ReactNode;
};
export function Field({
  label,
  description,
  error,
  id,
  className,
  children,
}: FieldProps) {
  return (
    <div className={cx("uiField", className)}>
      {label && (
        <label className="uiFieldLabel" htmlFor={id}>
          {label}
        </label>
      )}
      {description && (
        <div id={`${id}-description`} className="uiFieldDescription">
          {description}
        </div>
      )}
      {children}
      {error && (
        <div id={`${id}-error`} className="uiFieldError">
          {error}
        </div>
      )}
    </div>
  );
}
type InputProps = Omit<InputHTMLAttributes<HTMLInputElement>, "size"> & {
  size?: Size;
  rightSection?: ReactNode;
  rightSectionPointerEvents?: string;
};
const InputControl = forwardRef<HTMLInputElement, InputProps>(function Input(
  { size, rightSection, rightSectionPointerEvents, className, ...props },
  ref,
) {
  return (
    <div className={cx("uiInputWrapper", className)}>
      <input {...props} ref={ref} className="uiInput" data-size={size} />
      {rightSection && <span className="uiInputSection">{rightSection}</span>}
    </div>
  );
});
export const Input = Object.assign(InputControl, { Wrapper: Field });
export const TextInput = forwardRef<HTMLInputElement, InputProps & FieldProps>(
  function TextInput(
    { label, description, error, id: suppliedId, className, ...props },
    ref,
  ) {
    const generatedId = useId();
    const id = suppliedId ?? generatedId;
    return (
      <Field
        id={id}
        label={label}
        description={description}
        error={error}
        className={className}
      >
        <Input
          {...props}
          id={id}
          ref={ref}
          aria-invalid={!!error || undefined}
          aria-describedby={
            cx(description && `${id}-description`, error && `${id}-error`) ||
            undefined
          }
        />
      </Field>
    );
  },
);
export const UiInput = forwardRef<
  HTMLInputElement,
  InputHTMLAttributes<HTMLInputElement>
>(function UiInput({ size, type, ...props }, ref) {
  if (["checkbox", "radio", "file", "hidden", "color"].includes(type ?? ""))
    return <input {...props} type={type} ref={ref} size={size} />;
  return <Input {...props} type={type} ref={ref} />;
});
export const UiTextarea = forwardRef<
  HTMLTextAreaElement,
  React.TextareaHTMLAttributes<HTMLTextAreaElement>
>(function UiTextarea({ className, ...props }, ref) {
  return <textarea {...props} ref={ref} className={cx("uiInput", className)} />;
});
export function NumberInput({
  onChange,
  value,
  ...props
}: Omit<React.ComponentProps<typeof TextInput>, "value" | "onChange"> & {
  value: number | string;
  onChange: (value: number | string) => void;
}) {
  return (
    <TextInput
      {...props}
      type="number"
      value={value}
      onChange={(event) =>
        onChange(
          event.currentTarget.value === ""
            ? ""
            : Number(event.currentTarget.value),
        )
      }
    />
  );
}

type CheckProps = Omit<React.ComponentProps<typeof Check.Root>, "children"> & {
  label?: ReactNode;
  description?: ReactNode;
};
export function Checkbox({
  label,
  description,
  className,
  id: suppliedId,
  ...props
}: CheckProps) {
  const generatedId = useId();
  const id = suppliedId ?? generatedId;
  return (
    <div className={cx("uiCheckField", className)}>
      <Check.Root
        {...props}
        id={id}
        className="uiCheckbox"
        aria-describedby={description ? `${id}-description` : undefined}
      >
        <Check.Indicator>
          <CheckGlyph size={14} strokeWidth={2} aria-hidden />
        </Check.Indicator>
      </Check.Root>
      {label && (
        <label htmlFor={id}>
          {label}
          {description && <small id={`${id}-description`}>{description}</small>}
        </label>
      )}
    </div>
  );
}
export function Switch({
  label,
  id: suppliedId,
  ...props
}: React.ComponentProps<typeof SwitchPrimitive.Root> & { label?: ReactNode }) {
  const generatedId = useId();
  const id = suppliedId ?? generatedId;
  const control = (
    <SwitchPrimitive.Root
      {...props}
      id={id}
      className={cx("uiSwitch", props.className)}
    >
      <SwitchPrimitive.Thumb className="uiSwitchThumb" />
    </SwitchPrimitive.Root>
  );
  return label ? (
    <div className="uiCheckField">
      {control}
      <label htmlFor={id}>{label}</label>
    </div>
  ) : (
    control
  );
}
export const Radio = Object.assign(
  function Radio({
    label,
    description,
    id: suppliedId,
    ...props
  }: React.ComponentProps<typeof RadioGroup.Item> & {
    label?: ReactNode;
    description?: ReactNode;
  }) {
    const generatedId = useId();
    const id = suppliedId ?? generatedId;
    return (
      <div className="uiCheckField">
        <RadioGroup.Item
          {...props}
          id={id}
          className="uiRadio"
          aria-describedby={description ? `${id}-description` : undefined}
        >
          <RadioGroup.Indicator className="uiRadioIndicator" />
        </RadioGroup.Item>
        <label htmlFor={id}>
          {label}
          {description && <small id={`${id}-description`}>{description}</small>}
        </label>
      </div>
    );
  },
  {
    Group: function RadioChoices({
      label,
      value,
      onChange,
      children,
    }: {
      label?: ReactNode;
      value: string;
      onChange: (value: string) => void;
      children: ReactNode;
    }) {
      const id = useId();
      return (
        <div className="uiField">
          <div id={id} className="uiFieldLabel">
            {label}
          </div>
          <RadioGroup.Root
            value={value}
            onValueChange={onChange}
            aria-labelledby={id}
            className="uiStack"
          >
            {children}
          </RadioGroup.Root>
        </div>
      );
    },
  },
);
export type Option = { value: string; label: string; disabled?: boolean };
export type Options = (
  string | Option | { group: string; items: (string | Option)[] }
)[];
const option = (item: string | Option): Option =>
  typeof item === "string" ? { value: item, label: item } : item;
const flatOptions = (data: Options) =>
  data.flatMap((item) =>
    typeof item === "object" && "group" in item
      ? item.items.map(option)
      : [option(item)],
  );
export function SegmentedControl({
  data,
  value,
  onChange,
  disabled,
  size,
  className,
}: {
  data: (string | Option)[];
  value: string;
  onChange: (value: string) => void;
  disabled?: boolean;
  size?: Size;
  className?: string;
}) {
  return (
    <ToggleGroup.Root
      type="single"
      value={value}
      onValueChange={(next) => next && onChange(next)}
      disabled={disabled}
      className={cx("uiSegments", className)}
      data-size={size}
    >
      {data.map(option).map((item) => (
        <ToggleGroup.Item
          key={item.value}
          value={item.value}
          disabled={item.disabled}
          className="uiSegment"
          data-value={item.value}
        >
          {item.label}
        </ToggleGroup.Item>
      ))}
    </ToggleGroup.Root>
  );
}
export const Chip = Object.assign(
  ({
    value,
    children,
  }: {
    value: string;
    children: ReactNode;
    size?: Size;
  }) => (
    <ToggleGroup.Item value={value} className="uiChip">
      {children}
    </ToggleGroup.Item>
  ),
  {
    Group: ({
      value,
      onChange,
      children,
    }: {
      multiple?: boolean;
      value: string[];
      onChange: (value: string[]) => void;
      children: ReactNode;
    }) => (
      <ToggleGroup.Root
        type="multiple"
        value={value}
        onValueChange={onChange}
        className="uiGroup"
      >
        {children}
      </ToggleGroup.Root>
    ),
  },
);

export type SelectProps = FieldProps & {
  data?: Options;
  value?: string | null;
  onChange?: (value: string) => void;
  placeholder?: string;
  disabled?: boolean;
  searchable?: boolean;
  allowDeselect?: boolean;
  "aria-label"?: string;
  name?: string;
};
export const Select = forwardRef<HTMLButtonElement, SelectProps>(
  function Select(
    {
      data = [],
      value,
      onChange,
      label,
      description,
      error,
      id: suppliedId,
      className,
      placeholder,
      searchable,
      allowDeselect,
      ...props
    },
    ref,
  ) {
    const generatedId = useId();
    const id = suppliedId ?? generatedId;
    const layer = usePageLayer();
    const [open, setOpen] = useState(false);
    const options = flatOptions(data);
    const triggerProps = {
      ...props,
      id,
      ref,
      className: "uiInput uiSelectTrigger",
      "aria-invalid": !!error || undefined,
      "aria-describedby": description ? `${id}-description` : undefined,
    };
    const selected = options.find((item) => item.value === value);
    return (
      <Field
        id={id}
        label={label}
        description={description}
        error={error}
        className={cx("uiSelect", className)}
      >
        {searchable ? (
          <PopoverPrimitive.Root open={open} onOpenChange={setOpen}>
            <PopoverPrimitive.Trigger
              {...triggerProps}
              role="combobox"
              aria-label={
                props["aria-label"] ??
                (typeof label === "string" ? label : undefined)
              }
              aria-expanded={open}
            >
              {selected?.label ?? placeholder}
              <ChevronDown size={14} strokeWidth={1.75} aria-hidden />
            </PopoverPrimitive.Trigger>
            <PopoverPrimitive.Portal container={layer}>
              <PopoverPrimitive.Content
                className="uiSelectDropdown uiSearchSelect"
                align="start"
                sideOffset={4}
              >
                <Command>
                  <Command.Input
                    aria-label={
                      props["aria-label"] ??
                      (typeof label === "string" ? label : t("common.filter"))
                    }
                    className="uiInput"
                  />
                  <Command.List>
                    {options.map((item) => (
                      <Command.Item
                        key={item.value}
                        data-option-value={item.value}
                        value={item.value}
                        keywords={[item.label]}
                        disabled={item.disabled}
                        onSelect={() => {
                          onChange?.(item.value);
                          setOpen(false);
                        }}
                        className="uiSelectOption"
                      >
                        {item.label}
                      </Command.Item>
                    ))}
                  </Command.List>
                </Command>
              </PopoverPrimitive.Content>
            </PopoverPrimitive.Portal>
          </PopoverPrimitive.Root>
        ) : (
          <SelectPrimitive.Root
            value={value == null ? undefined : `value:${value}`}
            onValueChange={(next) => onChange?.(next.slice(6))}
            disabled={props.disabled}
            name={props.name}
          >
            <SelectPrimitive.Trigger {...triggerProps}>
              <SelectPrimitive.Value placeholder={placeholder} />
              <SelectPrimitive.Icon>
                <ChevronDown size={14} strokeWidth={1.75} aria-hidden />
              </SelectPrimitive.Icon>
            </SelectPrimitive.Trigger>
            <SelectPrimitive.Portal container={layer}>
              <SelectPrimitive.Content
                position="popper"
                sideOffset={4}
                className="uiSelectDropdown"
              >
                <SelectPrimitive.ScrollUpButton className="uiSelectScroll">
                  <ChevronUp size={14} strokeWidth={1.75} aria-hidden />
                </SelectPrimitive.ScrollUpButton>
                <SelectPrimitive.Viewport>
                  {data.map((item, index) =>
                    typeof item === "object" && "group" in item ? (
                      <SelectPrimitive.Group key={index}>
                        <SelectPrimitive.Label className="uiSelectGroupLabel">
                          {item.group}
                        </SelectPrimitive.Label>
                        {item.items.map(option).map(renderOption)}
                      </SelectPrimitive.Group>
                    ) : (
                      renderOption(option(item))
                    ),
                  )}
                </SelectPrimitive.Viewport>
                <SelectPrimitive.ScrollDownButton className="uiSelectScroll">
                  <ChevronDown size={14} strokeWidth={1.75} aria-hidden />
                </SelectPrimitive.ScrollDownButton>
              </SelectPrimitive.Content>
            </SelectPrimitive.Portal>
          </SelectPrimitive.Root>
        )}
      </Field>
    );
  },
);
function renderOption(item: Option) {
  return (
    <SelectPrimitive.Item
      key={item.value}
      value={`value:${item.value}`}
      disabled={item.disabled}
      className="uiSelectOption"
      data-option-value={item.value}
    >
      <SelectPrimitive.ItemText>{item.label}</SelectPrimitive.ItemText>
      <SelectPrimitive.ItemIndicator className="uiSelectIndicator">
        <CheckGlyph size={14} strokeWidth={1.75} aria-hidden />
      </SelectPrimitive.ItemIndicator>
    </SelectPrimitive.Item>
  );
}
export const UiSelect = forwardRef<
  HTMLButtonElement,
  Omit<SelectProps, "value"> & { value?: string | number }
>(function UiSelect({ value, ...props }, ref) {
  return (
    <Select
      {...props}
      ref={ref}
      value={value === undefined ? null : String(value)}
    />
  );
});
export function MultiSelect({
  data,
  value,
  onChange,
  maxValues,
  className,
  ...props
}: {
  data: Option[];
  value: string[];
  onChange: (value: string[]) => void;
  maxValues?: number;
  className?: string;
  "aria-label"?: string;
}) {
  const layer = usePageLayer();
  return (
    <PopoverPrimitive.Root>
      <PopoverPrimitive.Trigger
        className={cx("uiInput uiMultiSelect", className)}
        {...props}
      >
        {data
          .filter((item) => value.includes(item.value))
          .map((item) => (
            <span
              className="uiTag"
              data-variant="soft"
              data-role="accent"
              key={item.value}
            >
              {item.label}
            </span>
          ))}
        <ChevronDown size={14} strokeWidth={1.75} aria-hidden />
      </PopoverPrimitive.Trigger>
      <PopoverPrimitive.Portal container={layer}>
        <PopoverPrimitive.Content
          className="uiSelectDropdown"
          sideOffset={4}
          align="start"
        >
          <ToggleGroup.Root
            type="multiple"
            value={value}
            onValueChange={(next) => {
              if (!maxValues || next.length <= maxValues) onChange(next);
            }}
            aria-label={props["aria-label"]}
            className="uiMultiOptions"
          >
            {data.map((item) => (
              <ToggleGroup.Item
                value={item.value}
                key={item.value}
                className="uiSelectOption"
                disabled={
                  item.disabled ||
                  (!value.includes(item.value) &&
                    !!maxValues &&
                    value.length >= maxValues)
                }
              >
                {value.includes(item.value) && (
                  <CheckGlyph size={14} strokeWidth={1.75} aria-hidden />
                )}
                {item.label}
              </ToggleGroup.Item>
            ))}
          </ToggleGroup.Root>
        </PopoverPrimitive.Content>
      </PopoverPrimitive.Portal>
    </PopoverPrimitive.Root>
  );
}

// Radix has no file-picker primitive: the native input owns OS picker behaviour.
export function FileButton({
  accept,
  onChange,
  resetRef,
  disabled,
  children,
}: {
  accept?: string;
  onChange: (file: File | null) => void;
  resetRef?: RefObject<(() => void) | null>;
  disabled?: boolean;
  children: (props: { onClick: () => void; disabled?: boolean }) => ReactNode;
}) {
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (resetRef)
      resetRef.current = () => {
        if (input.current) input.current.value = "";
      };
    return () => {
      if (resetRef) resetRef.current = null;
    };
  }, [resetRef]);
  return (
    <>
      {children({ onClick: () => input.current?.click(), disabled })}
      <input
        ref={input}
        type="file"
        accept={accept}
        disabled={disabled}
        hidden
        onChange={(event) => {
          onChange(event.currentTarget.files?.[0] ?? null);
          event.currentTarget.value = "";
        }}
      />
    </>
  );
}
export function Progress({
  value,
  size,
  color,
  animated,
  radius,
  transitionDuration,
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement> & {
  value: number;
  size?: Size;
  color?: string;
  animated?: boolean;
  radius?: Size;
  transitionDuration?: number;
}) {
  return (
    <ProgressPrimitive.Root
      {...props}
      value={value}
      max={100}
      className={cx("uiProgress", className)}
      data-size={size}
      data-color={color}
    >
      <ProgressPrimitive.Indicator
        className="uiProgressIndicator"
        data-animated={animated || undefined}
        style={{
          transform: `translateX(-${100 - Math.max(0, Math.min(100, value))}%)`,
          transitionDuration: transitionDuration
            ? `${transitionDuration}ms`
            : undefined,
        }}
      />
    </ProgressPrimitive.Root>
  );
}
// Radix has no pagination primitive; native buttons retain form/keyboard semantics.
export function Pagination({
  value,
  onChange,
  total,
  size,
  ...props
}: Omit<React.HTMLAttributes<HTMLElement>, "onChange"> & {
  value: number;
  onChange: (value: number) => void;
  total: number;
  size?: Size;
}) {
  const pages = Array.from({ length: total }, (_, i) => i + 1).filter(
    (page) => page === 1 || page === total || Math.abs(page - value) <= 1,
  );
  return (
    <nav {...props} className="uiGroup uiPagination">
      <Button
        variant="default"
        disabled={value <= 1}
        onClick={() => onChange(value - 1)}
        aria-label={t("common.previousPage")}
      >
        <ChevronLeft size={14} strokeWidth={1.75} aria-hidden />
      </Button>
      {pages.map((page, index) => (
        <span key={page}>
          {index > 0 && page - pages[index - 1] > 1 && (
            <span className="uiPageGap">…</span>
          )}
          <Button
            variant={page === value ? "filled" : "default"}
            aria-current={page === value ? "page" : undefined}
            onClick={() => onChange(page)}
          >
            {page}
          </Button>
        </span>
      ))}
      <Button
        variant="default"
        disabled={value >= total}
        onClick={() => onChange(value + 1)}
        aria-label={t("common.nextPage")}
      >
        <ChevronRight size={14} strokeWidth={1.75} aria-hidden />
      </Button>
    </nav>
  );
}

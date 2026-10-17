import {
  ActionIcon,
  Box,
  Button as MantineButton,
  Input as MantineInput,
  MantineContext,
  Select,
  type SelectProps,
  UnstyledButton,
} from "@mantine/core";
import { forwardRef, useContext } from "react";

export const UiButton = forwardRef<
  HTMLButtonElement,
  React.ButtonHTMLAttributes<HTMLButtonElement>
>(function UiButton({ className = "", children, ...props }, ref) {
  const hasMantine = useContext(MantineContext) !== null;
  if (!hasMantine)
    return (
      <button ref={ref} className={className} {...props}>
        {children}
      </button>
    );

  const classes = className.split(/\s+/);
  if (classes.includes("iconButton")) {
    return (
      <ActionIcon
        ref={ref}
        className={className}
        variant="subtle"
        color="gray"
        size={30}
        {...props}
      >
        {children}
      </ActionIcon>
    );
  }
  if (classes.includes("textButton")) {
    return (
      <MantineButton
        ref={ref}
        className={className}
        variant="subtle"
        size="compact-sm"
        {...props}
      >
        {children}
      </MantineButton>
    );
  }
  if (classes.includes("button")) {
    const variant = classes.includes("primary")
      ? "filled"
      : classes.includes("quiet") || classes.includes("dangerText")
        ? "subtle"
        : "default";
    const color =
      classes.includes("danger") || classes.includes("dangerText")
        ? "red"
        : undefined;
    return (
      <MantineButton
        ref={ref}
        className={className}
        variant={variant}
        color={color}
        {...props}
      >
        {children}
      </MantineButton>
    );
  }
  return (
    <UnstyledButton ref={ref} className={className} {...props}>
      {children}
    </UnstyledButton>
  );
});

export const UiInput = forwardRef<
  HTMLInputElement,
  React.InputHTMLAttributes<HTMLInputElement>
>(function UiInput({ type, size: _nativeSize, ...props }, ref) {
  const hasMantine = useContext(MantineContext) !== null;
  if (!hasMantine) return <input ref={ref} type={type} {...props} />;

  if (["checkbox", "radio", "file", "hidden", "color"].includes(type ?? ""))
    return <Box component="input" ref={ref} type={type} {...props} />;
  return <MantineInput ref={ref} type={type} {...props} />;
});

type UiSelectProps = Omit<SelectProps, "value" | "onChange"> & {
  value?: string | number;
  onChange?: (value: string) => void;
};

/** The shell uses a themed Web combobox; option groups retain their keyboard order. */
export const UiSelect = forwardRef<HTMLInputElement, UiSelectProps>(
  function UiSelect({ value, onChange, className = "", ...props }, ref) {
    return (
      <Select
        {...props}
        ref={ref}
        className={`uiSelect ${className}`}
        value={value === undefined ? null : String(value)}
        onChange={(next) => {
          if (next !== null) onChange?.(next);
        }}
        classNames={{ dropdown: "uiSelectDropdown" }}
        allowDeselect={false}
        comboboxProps={{ withinPortal: true }}
      />
    );
  },
);

export const UiTextarea = forwardRef<
  HTMLTextAreaElement,
  React.TextareaHTMLAttributes<HTMLTextAreaElement>
>(function UiTextarea(props, ref) {
  const hasMantine = useContext(MantineContext) !== null;
  if (!hasMantine) return <textarea ref={ref} {...props} />;
  return <MantineInput component="textarea" multiline ref={ref} {...props} />;
});

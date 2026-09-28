import {
  ActionIcon,
  Box,
  Button as MantineButton,
  Input as MantineInput,
  MantineContext,
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

export const UiSelect = forwardRef<
  HTMLSelectElement,
  React.SelectHTMLAttributes<HTMLSelectElement>
>(function UiSelect({ size: _nativeSize, ...props }, ref) {
  const hasMantine = useContext(MantineContext) !== null;
  if (!hasMantine) return <select ref={ref} {...props} />;
  return <MantineInput component="select" ref={ref} {...props} />;
});

export const UiTextarea = forwardRef<
  HTMLTextAreaElement,
  React.TextareaHTMLAttributes<HTMLTextAreaElement>
>(function UiTextarea(props, ref) {
  const hasMantine = useContext(MantineContext) !== null;
  if (!hasMantine) return <textarea ref={ref} {...props} />;
  return <MantineInput component="textarea" ref={ref} {...props} />;
});

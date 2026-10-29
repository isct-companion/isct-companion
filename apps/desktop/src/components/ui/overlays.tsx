import { Dialog, DropdownMenu, Popover as PopoverPrimitive } from "radix-ui";
import {
  createContext,
  useContext,
  useState,
  type CSSProperties,
  type HTMLAttributes,
  type ReactNode,
} from "react";
import { usePageLayer } from "../../app/KeptPage";
import { t } from "../../i18n";
import { cx, spacing, type Size } from "./layout";
import { CloseButton } from "./controls";
import { createPortal } from "react-dom";

const focusedElement = () =>
  document.activeElement instanceof HTMLElement ? document.activeElement : null;

type ModalProps = {
  opened: boolean;
  onClose: () => void;
  title?: ReactNode;
  children: ReactNode;
  size?: Size | string;
  centered?: boolean;
  closeOnClickOutside?: boolean;
  closeOnEscape?: boolean;
  withCloseButton?: boolean;
  keepMounted?: boolean;
  keepMountedMode?: "display-none";
  classNames?: { body?: string };
  styles?: { body?: CSSProperties };
  overlayProps?: { backgroundOpacity?: number; blur?: number };
};
function DialogSurface({
  opened,
  onClose,
  title,
  children,
  size = "md",
  centered,
  closeOnClickOutside = true,
  closeOnEscape = true,
  withCloseButton = true,
  keepMounted,
  keepMountedMode,
  classNames,
  styles,
  overlayProps,
  drawer,
}: ModalProps & { drawer?: boolean }) {
  const layer = usePageLayer();
  // Where focus returns on close, read while rendering the open state: a field
  // with autoFocus takes focus as the dialog commits, before Radix's focus
  // scope looks, and Radix then skips onOpenAutoFocus.
  const [returnFocus, setReturnFocus] = useState(() =>
    opened ? focusedElement() : null,
  );
  const [wasOpened, setWasOpened] = useState(opened);
  if (opened !== wasOpened) {
    setWasOpened(opened);
    if (opened) setReturnFocus(focusedElement());
  }
  // A camera must keep its video/canvas refs while the dialog releases modality.
  // Radix changes its content implementation when modal changes; a stable portal
  // keeps the preview mounted while its host is moved to the new dialog body.
  const [retainedBody] = useState(() =>
    keepMounted ? document.createElement("div") : null,
  );
  const [attachBody] = useState(() => (host: HTMLDivElement | null) => {
    if (host && retainedBody) host.append(retainedBody);
  });
  const width =
    typeof size === "number"
      ? size
      : ((
          { xs: 320, sm: 380, md: 440, lg: 620, xl: 780 } as Record<
            string,
            number
          >
        )[size] ?? size);
  return (
    <>
      {retainedBody && createPortal(children, retainedBody)}
      <Dialog.Root
        open={opened}
        onOpenChange={(open) => {
          if (!open) onClose();
        }}
        modal={opened}
      >
        <Dialog.Portal container={layer} forceMount={keepMounted || undefined}>
          {opened && (
            <Dialog.Overlay
              className="uiDialogOverlay"
              style={{
                backgroundColor: `rgba(0,0,0,${overlayProps?.backgroundOpacity ?? 0.6})`,
                backdropFilter: overlayProps?.blur
                  ? `blur(${overlayProps.blur}px)`
                  : undefined,
              }}
            />
          )}
          <Dialog.Content
            forceMount={keepMounted || undefined}
            className={cx(
              "uiDialogContent",
              drawer && "uiDrawerContent",
              centered && "uiDialogCentered",
            )}
            style={{ width, display: !opened ? "none" : undefined }}
            aria-describedby={undefined}
            onEscapeKeyDown={(event) => {
              if (!closeOnEscape) event.preventDefault();
            }}
            onInteractOutside={(event) => {
              if (!closeOnClickOutside) event.preventDefault();
            }}
            onOpenAutoFocus={(event) => {
              if (!opened) event.preventDefault();
            }}
            onCloseAutoFocus={(event) => {
              // These controlled dialogs have no Radix Trigger to restore focus to.
              event.preventDefault();
              if (returnFocus?.isConnected)
                returnFocus.focus({ preventScroll: true });
            }}
          >
            <header className="uiDialogHeader">
              <Dialog.Title
                className={cx("uiDialogTitle", !title && "uiVisuallyHidden")}
              >
                {title ?? t("ui.confirm")}
              </Dialog.Title>
              {withCloseButton && (
                <Dialog.Close asChild>
                  <CloseButton aria-label={t("common.close")} />
                </Dialog.Close>
              )}
            </header>
            <div
              className={cx("uiDialogBody", classNames?.body)}
              style={styles?.body}
            >
              {retainedBody ? <div ref={attachBody} /> : children}
            </div>
          </Dialog.Content>
        </Dialog.Portal>
      </Dialog.Root>
    </>
  );
}
export const Modal = (props: ModalProps) => <DialogSurface {...props} />;
export const Drawer = ({
  position,
  ...props
}: ModalProps & { position?: "right" }) => <DialogSurface {...props} drawer />;

type Placement =
  "top" | "bottom" | "left" | "right" | "bottom-start" | "bottom-end";
const placement = (value: Placement) => ({
  side: value.split("-")[0] as "top" | "bottom" | "left" | "right",
  align: (value.endsWith("-end")
    ? "end"
    : value.endsWith("-start")
      ? "start"
      : "center") as "start" | "center" | "end",
});
const Floating = createContext<{
  position: Placement;
  offset: number;
  width?: number;
  withArrow?: boolean;
  closeOnClickOutside?: boolean;
  onDismiss?: () => void;
}>({ position: "bottom", offset: 4 });
type FloatingProps = {
  children: ReactNode;
  opened?: boolean;
  onChange?: (open: boolean) => void;
  position?: Placement;
  offset?: number;
  width?: number;
  withinPortal?: boolean;
  withArrow?: boolean;
  arrowSize?: number;
  shadow?: string;
  trapFocus?: boolean;
  closeOnClickOutside?: boolean;
  onDismiss?: () => void;
};
function PopoverRoot({
  children,
  opened,
  onChange,
  position = "bottom",
  offset = 4,
  width,
  withArrow,
  closeOnClickOutside,
  onDismiss,
  trapFocus,
}: FloatingProps) {
  return (
    <Floating.Provider
      value={{
        position,
        offset,
        width,
        withArrow,
        closeOnClickOutside,
        onDismiss,
      }}
    >
      <PopoverPrimitive.Root
        open={opened}
        onOpenChange={
          onChange ??
          ((open) => {
            if (!open) onDismiss?.();
          })
        }
        modal={trapFocus}
      >
        {children}
      </PopoverPrimitive.Root>
    </Floating.Provider>
  );
}
function PopoverContent({
  p,
  className,
  ...props
}: HTMLAttributes<HTMLDivElement> & { p?: Size }) {
  const layer = usePageLayer();
  const config = useContext(Floating);
  return (
    <PopoverPrimitive.Portal container={layer}>
      <PopoverPrimitive.Content
        {...props}
        {...placement(config.position)}
        sideOffset={config.offset}
        className={cx("uiPopover", className)}
        style={{ width: config.width, padding: spacing(p), ...props.style }}
        onOpenAutoFocus={(event) => event.preventDefault()}
        onEscapeKeyDown={() => config.onDismiss?.()}
        onInteractOutside={(event) => {
          if (config.closeOnClickOutside === false) event.preventDefault();
        }}
      >
        {props.children}
        {config.withArrow && (
          <PopoverPrimitive.Arrow
            className="uiPopoverArrow"
            width={10}
            height={5}
          />
        )}
      </PopoverPrimitive.Content>
    </PopoverPrimitive.Portal>
  );
}
export const Popover = Object.assign(PopoverRoot, {
  Target: ({ children }: { children: ReactNode }) => (
    <PopoverPrimitive.Trigger asChild>{children}</PopoverPrimitive.Trigger>
  ),
  Dropdown: PopoverContent,
});
function MenuRoot({
  children,
  opened,
  onChange,
  position = "bottom",
  offset = 4,
  width,
}: FloatingProps) {
  return (
    <Floating.Provider value={{ position, offset, width }}>
      <DropdownMenu.Root open={opened} onOpenChange={onChange}>
        {children}
      </DropdownMenu.Root>
    </Floating.Provider>
  );
}
function MenuContent({ className, ...props }: HTMLAttributes<HTMLDivElement>) {
  const layer = usePageLayer();
  const config = useContext(Floating);
  return (
    <DropdownMenu.Portal container={layer}>
      <DropdownMenu.Content
        {...props}
        {...placement(config.position)}
        sideOffset={config.offset}
        className={cx("uiMenu", className)}
        style={{ minWidth: config.width, ...props.style }}
      />
    </DropdownMenu.Portal>
  );
}
export const Menu = Object.assign(MenuRoot, {
  Target: ({ children }: { children: ReactNode }) => (
    <DropdownMenu.Trigger asChild>{children}</DropdownMenu.Trigger>
  ),
  Dropdown: MenuContent,
  Item: ({
    leftSection,
    onClick,
    ...props
  }: Omit<React.ComponentProps<typeof DropdownMenu.Item>, "onClick"> & {
    leftSection?: ReactNode;
    onClick?: () => void;
  }) => (
    <DropdownMenu.Item
      {...props}
      className={cx("uiMenuItem", props.className)}
      onSelect={onClick}
    >
      {leftSection && <span>{leftSection}</span>}
      {props.children}
    </DropdownMenu.Item>
  ),
});

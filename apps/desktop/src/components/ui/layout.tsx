import {
  createElement,
  type CSSProperties,
  type HTMLAttributes,
  type ReactNode,
} from "react";

export type Size = "xs" | "sm" | "md" | "lg" | "xl" | number;
export const spacing = (value?: Size) =>
  typeof value === "number"
    ? value
    : value
      ? {
          xs: "0.625rem",
          sm: "0.75rem",
          md: "1rem",
          lg: "1.25rem",
          xl: "2rem",
        }[value]
      : undefined;
export const cx = (...values: (string | false | null | undefined | 0 | 0n)[]) =>
  values.filter(Boolean).join(" ");
export type SurfaceProps = HTMLAttributes<HTMLElement> & {
  component?: "div" | "section" | "aside" | "header" | "span" | "button";
  p?: Size;
  px?: Size;
  py?: Size;
  mt?: Size;
  mah?: number;
  radius?: Size;
  shadow?: string;
  withBorder?: boolean;
};
export function surfaceStyle({
  p,
  px,
  py,
  mt,
  mah,
  style,
}: SurfaceProps): CSSProperties {
  return {
    padding: spacing(p),
    paddingInline: spacing(px),
    paddingBlock: spacing(py),
    marginTop: spacing(mt),
    maxHeight: mah,
    ...style,
  };
}
export function Paper({
  component = "div",
  p,
  px,
  py,
  mt,
  mah,
  radius,
  shadow,
  withBorder,
  className,
  style,
  ...props
}: SurfaceProps) {
  return createElement(component, {
    ...props,
    className: cx("uiPaper", className),
    style: surfaceStyle({ p, px, py, mt, mah, style }),
  });
}
type LayoutProps = SurfaceProps & {
  gap?: Size;
  justify?: CSSProperties["justifyContent"];
  align?: CSSProperties["alignItems"];
  wrap?: CSSProperties["flexWrap"];
  grow?: boolean;
};
function Layout(
  {
    component = "div",
    gap = "md",
    justify,
    align,
    wrap,
    grow,
    p,
    px,
    py,
    mt,
    mah,
    radius,
    shadow,
    withBorder,
    className,
    style,
    ...props
  }: LayoutProps,
  stack: boolean,
) {
  return createElement(component, {
    ...props,
    className: cx(stack ? "uiStack" : "uiGroup", className),
    "data-grow": grow || undefined,
    style: {
      gap: spacing(gap),
      justifyContent: justify,
      alignItems: align,
      flexWrap: wrap,
      ...surfaceStyle({ p, px, py, mt, mah, style }),
    },
  });
}
export const Group = (props: LayoutProps) => Layout(props, false);
export const Stack = (props: LayoutProps) => Layout(props, true);
export function Text({
  size = "md",
  fw,
  c,
  style,
  className,
  ...props
}: HTMLAttributes<HTMLDivElement> & {
  size?: Size;
  fw?: CSSProperties["fontWeight"];
  c?: string;
}) {
  return (
    <div
      {...props}
      className={cx("uiText", className)}
      data-size={size}
      data-color={c}
      style={{ fontWeight: fw, ...style }}
    />
  );
}
export function Badge({
  component = "span",
  color = "campus",
  variant,
  children,
  className,
  ...props
}: Omit<SurfaceProps, "color"> & {
  color?: string;
  variant?: string;
  type?: "button";
  children?: ReactNode;
}) {
  return createElement(
    component,
    { ...props, "data-color": color, className: cx("uiBadge", className) },
    children,
  );
}
export function Alert({
  color,
  mt,
  children,
  className,
  ...props
}: HTMLAttributes<HTMLDivElement> & { color?: string; mt?: Size }) {
  return (
    <div
      {...props}
      className={cx("uiAlert", className)}
      data-color={color}
      style={{ marginTop: spacing(mt), ...props.style }}
    >
      {children}
    </div>
  );
}

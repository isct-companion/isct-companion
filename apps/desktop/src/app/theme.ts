import {
  createTheme,
  type CSSVariablesResolver,
  type MantineColorsTuple,
} from "@mantine/core";
import tokens from "../generated/design/tokens.json";
import { t } from "../i18n";

// Mantine queries the system scheme even when the app has an explicit scheme.
// Browsers implement matchMedia; lightweight DOM test environments may not.
if (typeof window !== "undefined" && typeof window.matchMedia !== "function") {
  window.matchMedia = (query: string) =>
    ({
      matches: false,
      media: query,
      onchange: null,
      addListener: () => undefined,
      removeListener: () => undefined,
      addEventListener: () => undefined,
      removeEventListener: () => undefined,
      dispatchEvent: () => false,
    }) as MediaQueryList;
}

// Mantine's tonal scale and font come from design/tokens (yarn design:tokens).
const campusBlue = Object.values(
  tokens["accent-scale"],
) as unknown as MantineColorsTuple;

export const appTheme = createTheme({
  primaryColor: "campus",
  colors: { campus: campusBlue },
  fontFamily: tokens.font.family.sans,
  defaultRadius: "md",
  cursorType: "pointer",
  focusRing: "auto",
  components: {
    Button: {
      defaultProps: { size: "xs", radius: "md" },
    },
    ActionIcon: {
      defaultProps: { size: "lg", radius: "md", variant: "subtle" },
    },
    Input: {
      defaultProps: { size: "sm", radius: "md" },
    },
    TextInput: {
      defaultProps: { size: "sm", radius: "md" },
    },
    PasswordInput: {
      defaultProps: { size: "sm", radius: "md" },
    },
    // Resolved on each render so the label follows the current language.
    Modal: {
      defaultProps: () => ({ closeButtonProps: { "aria-label": t("Close") } }),
    },
    Drawer: {
      defaultProps: () => ({ closeButtonProps: { "aria-label": t("Close") } }),
    },
  },
});

// Keep the public CSS variables stable; per-site CSS and hooks may rely on them.
export const cssVariablesResolver: CSSVariablesResolver = () => ({
  variables: {
    "--mantine-primary-color-filled": "var(--accent)",
    "--mantine-primary-color-filled-hover":
      "color-mix(in srgb, var(--accent) 88%, black)",
    "--mantine-color-body": "var(--bg)",
    "--mantine-color-text": "var(--ink)",
  },
  light: {},
  dark: {},
});

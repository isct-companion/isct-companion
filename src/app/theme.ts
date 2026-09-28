import {
  createTheme,
  type CSSVariablesResolver,
  type MantineColorsTuple,
} from "@mantine/core";

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

const campusBlue: MantineColorsTuple = [
  "#eef3ff",
  "#dce7ff",
  "#b8cdff",
  "#8faeff",
  "#6b94f8",
  "#527ff0",
  "#416fE5",
  "#315fd0",
  "#2853bb",
  "#21479f",
];

export const appTheme = createTheme({
  primaryColor: "campus",
  colors: { campus: campusBlue },
  fontFamily:
    "Inter, -apple-system, BlinkMacSystemFont, 'Hiragino Sans', 'Yu Gothic UI', system-ui, sans-serif",
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
  },
});

// Keep the public CSS variables stable so user-injected App CSS remains valid.
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

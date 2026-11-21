/** Tauri errors may be native typed failures or JavaScript Errors. */
export const errorText = (error: unknown, { nativeMessage = true } = {}) =>
  error instanceof Error
    ? error.message
    : typeof error === "string"
      ? error
      : nativeMessage &&
          error &&
          typeof error === "object" &&
          "message" in error &&
          typeof error.message === "string"
        ? error.message
        : undefined;

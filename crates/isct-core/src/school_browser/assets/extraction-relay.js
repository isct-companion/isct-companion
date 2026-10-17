(() => {
  const relay = __ISCT_RELAY__;
  const run = __ISCT_RUN__;
  Promise.resolve()
    .then(() => {
      // Extractors are expression statements, including comments and a final
      // semicolon. An initializer accepts that syntax; a call argument does not.
      const extracted = __ISCT_EXTRACTOR__;
      return extracted;
    })
    .catch(() => ({
      status: "error",
      sourceUrl: location.href,
      error: "School page extraction failed.",
      message: "School page extraction failed.",
    }))
    .then((value) => {
      if (value && typeof value === "object") value._isctExtractionRun = run;
      location.href =
        relay +
        btoa(unescape(encodeURIComponent(JSON.stringify(value))))
          .replace(/\+/g, "-")
          .replace(/\//g, "_")
          .replace(/=+$/, "");
    });
})();

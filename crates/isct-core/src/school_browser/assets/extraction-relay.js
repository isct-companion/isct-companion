(() => {
  const relay = __ISCT_RELAY__;
  const run = __ISCT_RUN__;
  const part = __ISCT_PART__;
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
      const encoded = btoa(unescape(encodeURIComponent(JSON.stringify(value))))
        .replace(/\+/g, "-")
        .replace(/\//g, "_")
        .replace(/=+$/, "");
      // Chromium drops navigations to URLs over 2 MiB, so the result crosses
      // in parts. Core asks for each next part once it holds the previous one.
      const count = Math.max(1, Math.ceil(encoded.length / part));
      const send = (index) => {
        location.href = `${relay}${run}/${count}/${index}/${encoded.slice(index * part, (index + 1) * part)}`;
      };
      globalThis.__isctRelayPart = (forRun, index) => {
        if (forRun === run && index > 0 && index < count) setTimeout(() => send(index), 0);
      };
      send(0);
    });
})();

(() => {
  const relay = __ISCT_RELAY__;
  const run = __ISCT_RUN__;
  const part = __ISCT_PART__;
  const maxBytes = __ISCT_MAX__;
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
      const json = JSON.stringify(value) ?? "";
      const rawPart = Math.floor(part / 4) * 3;
      const byteWidth = (codePoint) =>
        codePoint <= 0x7f ? 1 : codePoint <= 0x7ff ? 2 : codePoint <= 0xffff ? 3 : 4;
      let totalBytes = 0;
      let count = 0;
      let measuredPartBytes = 0;
      for (let offset = 0; offset < json.length; ) {
        const codePoint = json.codePointAt(offset);
        const codeUnits = codePoint > 0xffff ? 2 : 1;
        const width = byteWidth(codePoint);
        totalBytes += width;
        if (measuredPartBytes > 0 && measuredPartBytes + width > rawPart) {
          count += 1;
          measuredPartBytes = 0;
        }
        measuredPartBytes += width;
        offset += codeUnits;
      }
      count = Math.max(1, count + 1);

      // Reject a known-oversized payload before allocating any encoded byte
      // buffer or sending the payload through the browser navigation relay.
      if (totalBytes > maxBytes) {
        location.href = `${relay}${run}/0/0/oversized-${totalBytes}`;
        return;
      }

      // Chromium drops navigations to URLs over 2 MiB, so transfer one
      // URL-sized part at a time and retain only that part's byte buffer.
      const encode = (chunk) => {
        let binary = "";
        for (let offset = 0; offset < chunk.length; offset += 32768) {
          binary += String.fromCharCode(...chunk.subarray(offset, offset + 32768));
        }
        return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
      };
      let textOffset = 0;
      const send = (index) => {
        const start = textOffset;
        let chunkBytes = 0;
        while (textOffset < json.length) {
          const codePoint = json.codePointAt(textOffset);
          const width = byteWidth(codePoint);
          if (chunkBytes > 0 && chunkBytes + width > rawPart) break;
          chunkBytes += width;
          textOffset += codePoint > 0xffff ? 2 : 1;
        }
        const chunk = new TextEncoder().encode(json.slice(start, textOffset));
        location.href = `${relay}${run}/${count}/${index}/${encode(chunk)}`;
      };
      globalThis.__isctRelayPart = (forRun, index) => {
        if (forRun !== run || index <= 0) return;
        if (index >= count) {
          delete globalThis.__isctRelayPart;
          return;
        }
        setTimeout(() => send(index), 0);
      };
      send(0);
    });
})();

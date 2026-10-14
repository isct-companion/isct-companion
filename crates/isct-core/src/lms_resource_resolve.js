({ page, responseUrl, cmid, module }) => {
  const login = Boolean(
    page.querySelector('input[type="password"], form[action*="login" i]'),
  );
  if (login) return { kind: "login" };
  if (!page.body?.classList.contains(`cmid-${cmid}`))
    return { kind: "unresolved" };

  const absolute = (raw) => {
    try {
      return new URL(raw, responseUrl);
    } catch {
      return null;
    }
  };
  const responseOrigin = absolute(responseUrl)?.origin;
  const pluginFile = (raw) => {
    const url = raw && absolute(raw);
    return url && url.origin === responseOrigin &&
      url.pathname.startsWith("/2025/pluginfile.php/") ? url.href : null;
  };

  if (module === "folder") {
    const files = [...page.querySelectorAll('.foldertree a[href*="/pluginfile.php/"]')]
      .map((link) => {
        const url = pluginFile(link.getAttribute("href"));
        return url ? { name: link.textContent.trim() || new URL(url).pathname.split("/").pop(), url } : null;
      })
      .filter(Boolean);
    return files.length ? { kind: "folder", files } : { kind: "unresolved" };
  }

  if (module === "url") {
    const link = page.querySelector('#region-main a[href^="https://"]');
    const url = link && absolute(link.getAttribute("href"));
    if (!url) return { kind: "unresolved" };
    const file = pluginFile(url.href);
    return file ? { kind: "file", url: file } : { kind: "external", url: url.href };
  }

  if (module === "resource") {
    for (const [selector, attribute] of [
      [".resourcecontent img", "src"],
      [".resourcecontent object", "data"],
      [".resourcecontent iframe", "src"],
      [".resourcecontent video source, .resourcecontent audio source", "src"],
      [".resourceworkaround a", "href"],
      [".resourcecontent a", "href"],
    ]) {
      const url = pluginFile(page.querySelector(selector)?.getAttribute(attribute));
      if (url) return { kind: "file", url };
    }
  }
  return { kind: "unresolved" };
}

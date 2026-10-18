// Shared reader for live pages and fetched search responses. No calendar is
// inferred: preserve the school's current notice, including listed periods.
globalThis.__isctKyomuMaintenance = (root) => {
  const clean = (value) =>
    String(value || "")
      .normalize("NFKC")
      .replace(/\s+/g, " ")
      .trim();
  const headings = [
    ...root.querySelectorAll(
      "h1,h2,h3,h4,[role=heading],title,strong,b,p,span",
    ),
  ];
  const heading = headings.find((node) =>
    /(?:ただいま|現在).*(?:メンテナンス中)|under maintenance|maintenance in progress/i.test(
      clean(node.textContent),
    ),
  );
  if (!heading) return null;
  // Require a current outage heading. A future notice in a working search
  // page, or a course mentioning maintenance, must not stop the service.
  const region =
    heading.closest("main,article,[role=main]") ||
    root.body ||
    root.documentElement;
  const details = clean(region?.innerText || region?.textContent);
  const notes = [...region.querySelectorAll("textarea")]
    .map((node) => clean(node.value || node.textContent))
    .filter(Boolean);
  return [details, ...notes.filter((note) => !details.includes(note))]
    .filter(Boolean)
    .join(" ")
    .slice(0, 1800);
};

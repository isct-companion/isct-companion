(() => {
  // A target="_blank" link first goes through the view's navigation policy,
  // which only admits official school pages, so a link to any other site was
  // dropped before the app could route it. Every cross-origin new-window link
  // goes through window.open instead, which the app routes on its own: school
  // pages to their service, other pages to the system browser. Runs at
  // document start in every frame of the school views.
  window.addEventListener("click", (event) => {
    if (
      event.defaultPrevented ||
      event.button !== 0 ||
      event.metaKey ||
      event.altKey ||
      event.ctrlKey ||
      event.shiftKey
    )
      return;
    const link = event
      .composedPath()
      .find((node) => node instanceof Node && node.nodeName === "A");
    if (!link || !link.href || link.target.toLowerCase() !== "_blank") return;
    let url;
    try {
      url = new URL(link.href);
    } catch {
      return;
    }
    if (
      url.origin === location.origin ||
      (url.protocol !== "https:" && url.protocol !== "http:")
    )
      return;
    event.preventDefault();
    window.open(url.href, "_blank", "noopener");
  });
})();

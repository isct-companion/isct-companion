(() => {
  // A target="_blank" link first goes through the view's navigation policy,
  // must route the link in the owning view. Every cross-origin new-window link
  // goes through window.open instead: official services use their own view,
  // external pages remain in the user's visible browser workspace. Runs at
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

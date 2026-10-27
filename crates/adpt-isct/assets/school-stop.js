// Ends an abandoned sign-in or session check in this view: no further loads,
// and Core's scripts on the page stop acting on it.
(() => {
  window.stop();
  dispatchEvent(new Event("isct-companion-stop"));
})();

(() => {
  const scripts = __ISCT_SUPPORT_SCRIPTS__;
  const install = (win) => {
    try {
      if (win.__isctCompanionAndroidSupportInstalled) return;
      win.__isctCompanionAndroidSupportInstalled = true;
      for (const script of scripts) win.eval(script);
      // Android injects after load. Teach existing and future same-origin
      // frames the popup opener/close and school-link behavior as well.
      win.document.addEventListener("load", (event) => {
        if (event.target?.tagName === "IFRAME") install(event.target.contentWindow);
      }, true);
      for (const frame of win.document.querySelectorAll("iframe")) {
        install(frame.contentWindow);
      }
    } catch {
      // A cross-origin frame cannot access its parent document or scripts.
    }
  };
  install(window);
})();

(function enrichCourseContent(page, state, baseUrl, boundedBody) {
  const clean = node => {
    const copy = node.cloneNode(true);
    for (const element of [...copy.getElementsByTagName('*')]) {
      if (['SCRIPT','STYLE','INPUT','BUTTON','LABEL','FORM'].includes(element.tagName) || element.getAttribute('aria-hidden') === 'true') {
        element.remove(); continue;
      }
      for (const attr of ['href','src']) if (element.hasAttribute(attr)) {
        try { element.setAttribute(attr, new URL(element.getAttribute(attr), baseUrl).href); }
        catch { element.removeAttribute(attr); }
      }
      if (element.tagName === 'A' && element.hasAttribute('href')) {
        const target = new URL(element.getAttribute('href'), baseUrl);
        const current = new URL(baseUrl);
        if (target.pathname === current.pathname && target.search === current.search && (target.hash || target.href.endsWith('#'))) { element.remove(); continue; }
      }
      if (element.tagName === 'IMG' && element.getAttribute('src')?.includes('/theme/image.php/')) element.remove();
    }
    return copy;
  };
  const meaningful = node => node.textContent.trim() || [...node.getElementsByTagName('*')].some(el => el.tagName === 'IMG' || el.tagName === 'A' && el.hasAttribute('href'));
  // Activity identity comes from Moodle's API, not the theme's content classes.
  for (const cm of state.cm) {
    const node = page.getElementById(cm.anchor || 'module-' + cm.id);
    if (!node) continue;
    const copy = clean(node);
    for (const link of [...copy.getElementsByTagName('a')]) {
      if (cm.url && link.getAttribute('href') === new URL(cm.url, baseUrl).href) link.remove();
    }
    if (meaningful(copy)) cm.bodyHtml = boundedBody(copy.innerHTML);
  }
  for (const section of state.section) {
    const node = page.getElementById(section.anchor || 'section-' + section.number);
    if (!node) continue;
    const copy = clean(node);
    for (const element of [...copy.getElementsByTagName('*')]) {
      // Strip complete activities and section navigation; what remains is the
      // teacher's summary, including text, tables and embedded attachments.
      if (/^module-\d+$/.test(element.id)) element.remove();
      if (element.tagName === 'A' && new URL(element.getAttribute('href') || baseUrl, baseUrl).pathname.endsWith('/course/section.php')) {
        const heading = element.closest('h1,h2,h3,h4,h5,h6');
        (heading || element).remove();
      }
    }
    if (meaningful(copy)) section.bodyHtml = boundedBody(copy.innerHTML);
  }
})

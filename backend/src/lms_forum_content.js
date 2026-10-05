(function forumContent(page, baseUrl) {
  const result = [];
  for (const post of page.getElementsByTagName('*')) {
    if (!/^p\d+$/.test(post.id)) continue;
    const content = page.getElementById('post-content-' + post.id.slice(1));
    if (!content) continue;
    const article = page.createElement('article');
    article.id = 'announcement-' + post.id;
    article.setAttribute('data-source-url', baseUrl);
    const heading = post.getElementsByTagName('h3')[0];
    if (heading) {
      const title = page.createElement('h5');
      title.textContent = heading.textContent;
      article.append(title);
    }
    const time = post.getElementsByTagName('time')[0];
    if (time) { const date = page.createElement('time'); date.textContent = time.textContent; article.append(date); }
    const body = content.cloneNode(true);
    for (const element of body.getElementsByTagName('*')) {
      for (const attr of ['src', 'href']) if (element.hasAttribute(attr)) {
        try { element.setAttribute(attr, new URL(element.getAttribute(attr), baseUrl).href); }
        catch { element.removeAttribute(attr); }
      }
    }
    article.append(body);
    // Attachments outside the post body remain part of this announcement.
    for (const link of post.getElementsByTagName('a')) {
      const href = new URL(link.getAttribute('href') || '', baseUrl);
      if (href.pathname.includes('/pluginfile.php/') && !content.contains(link)) {
        const attachment = link.cloneNode(true); attachment.setAttribute('href', href.href); article.append(attachment);
      }
    }
    result.push(article.outerHTML);
  }
  return result.join('\n');
})

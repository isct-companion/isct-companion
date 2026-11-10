import { formatJst } from "../../lib/civil-date";
import { extension, fileFamily, previewKind } from "./file-types";
import { t } from "../../i18n";
/** Render a bounded, inert subset of teacher HTML. No scripts, forms or styles. */
export function safeContent(html: string, language?: "ja" | "en"): string {
  const source = new DOMParser().parseFromString(
    html.slice(0, 100000),
    "text/html",
  );
  source
    .querySelectorAll(
      "script,style,iframe,object,embed,form,input,button,textarea,select,svg",
    )
    .forEach((n) => n.remove());
  // Cached imports can still contain Moodle's own expand/collapse controls.
  for (const link of [...source.getElementsByTagName("a")]) {
    const href = safeUrl(link.getAttribute("href"));
    if (!href) continue;
    const url = new URL(href);
    if (
      url.hostname === "lms.s.isct.ac.jp" &&
      url.pathname.endsWith("/course/view.php") &&
      (href.endsWith("#") || /^#coursecontentcollapse\d+$/.test(url.hash))
    )
      link.remove();
  }
  // Only explicit block language metadata can filter arbitrary content.
  if (language && source.body.querySelector(`[lang="${language}"]`))
    source.body.querySelectorAll("[lang]").forEach((n) => {
      const lang = n.getAttribute("lang")?.toLowerCase();
      if ((lang === "ja" || lang === "en") && lang !== language) n.remove();
    });
  const out = document.createElement("div");
  const tags = new Set([
    "ARTICLE",
    "TIME",
    "P",
    "DIV",
    "SPAN",
    "BR",
    "B",
    "STRONG",
    "I",
    "EM",
    "U",
    "S",
    "UL",
    "OL",
    "LI",
    "TABLE",
    "THEAD",
    "TBODY",
    "TR",
    "TH",
    "TD",
    "PRE",
    "CODE",
    "BLOCKQUOTE",
    "H2",
    "H3",
    "H4",
    "H5",
    "HR",
    "A",
  ]);
  const copy = (node: Node, parent: HTMLElement) => {
    if (node.nodeType === Node.TEXT_NODE) {
      parent.append(document.createTextNode(node.textContent ?? ""));
      return;
    }
    if (!(node instanceof Element)) return;
    if (node.tagName === "IMG") {
      const url = safeUrl(node.getAttribute("src"));
      if (url && !new URL(url).pathname.includes("/theme/image.php/")) {
        const link = document.createElement("a");
        link.href = url;
        link.className = "lmsAttachmentCard";
        link.textContent = node.getAttribute("alt") || t("lms.image");
        const placeholder = document.createElement("span");
        placeholder.dataset.imageSrc = url;
        placeholder.dataset.imageAlt = link.textContent;
        if (parent.tagName === "A") {
          parent.classList.add("lmsImageCard");
          parent.append(placeholder, caption(link.textContent));
        } else {
          link.classList.add("lmsImageCard");
          link.replaceChildren(placeholder, caption(link.textContent));
          parent.append(link);
        }
      }
      return;
    }
    const el = tags.has(node.tagName)
      ? document.createElement(node.tagName.toLowerCase())
      : parent;
    if (el !== parent) {
      if (node.tagName === "ARTICLE") el.className = "lmsAnnouncement";
      if (node.tagName === "A") {
        const url = safeUrl(node.getAttribute("href"));
        if (url) {
          el.setAttribute("href", url);
          if (new URL(url).pathname.includes("/pluginfile.php/"))
            el.className = "lmsAttachmentCard";
        }
      }
      for (const attr of ["colspan", "rowspan"]) {
        const value = node.getAttribute(attr);
        if (/^\d{1,2}$/.test(value ?? "")) el.setAttribute(attr, value!);
      }
      // Core marks where the course is in a lesson schedule table.
      if (node.tagName === "TR")
        for (const mark of node.classList)
          if (/^lmsRow(?:Past|Now|Off)$/.test(mark)) el.classList.add(mark);
      parent.append(el);
    }
    node.childNodes.forEach((child) => copy(child, el));
    if (node.tagName === "A" && el !== parent) {
      const url = el.getAttribute("href");
      if (
        url &&
        previewKind(url) === "image" &&
        !el.querySelector("[data-image-src]")
      ) {
        const placeholder = document.createElement("span");
        placeholder.dataset.imageSrc = url;
        placeholder.dataset.imageAlt = el.textContent || t("lms.image");
        el.classList.add("lmsImageCard", "lmsAttachmentCard");
        el.replaceChildren(placeholder, caption(placeholder.dataset.imageAlt));
      } else if (
        url &&
        el.classList.contains("lmsAttachmentCard") &&
        !el.classList.contains("lmsImageCard")
      ) {
        fileCard(el, url);
      }
    }
  };
  source.body.childNodes.forEach((node) => copy(node, out));
  fileLists(out);
  if (language) {
    // Narrow fallback: complete paired, labelled lines only. Keep mixed or
    // unlabelled content unchanged, including tables, code and continuation.
    out.querySelectorAll("div,p").forEach((el) => {
      if (el.querySelector("div,p,table,pre,code,ul,ol,img")) return;
      const lines = el.innerHTML
        .split(/<br\s*\/?\s*>/i)
        .filter((line) => line.trim());
      const languages = lines.map((line) => {
        const text =
          new DOMParser()
            .parseFromString(line, "text/html")
            .body.textContent?.trim() ?? "";
        if (/^(授業計画|課題|連絡先|オフィスアワー)\s*[：:]/.test(text))
          return "ja";
        if (
          /^(Course schedule|Objectives|Contact|Office\s*hours)\s*:/i.test(text)
        )
          return "en";
        return undefined;
      });
      if (
        languages.includes("ja") &&
        languages.includes("en") &&
        languages.every(Boolean)
      )
        el.innerHTML = lines
          .filter((_, index) => languages[index] === language)
          .join("<br>");
    });
  }
  return out.innerHTML;
}

function caption(text: string) {
  const span = document.createElement("span");
  span.className = "lmsImageCaption";
  span.textContent = text;
  return span;
}

/** Attachment link → card: type tile, name, and a meta line (type · date). */
function fileCard(link: HTMLElement, url: string) {
  const name = document.createElement("span");
  name.className = "lmsAttachmentName";
  name.textContent =
    link.textContent?.trim() || url.split("/").pop() || t("lms.file");
  const meta = document.createElement("small");
  meta.textContent = extension(url).toUpperCase() || "FILE";
  link.dataset.family = fileFamily(url);
  link.replaceChildren(name, meta);
}

/**
 * Moodle lists attachments as `ul > li > (icon + link, upload time)`. A list
 * holding only attachment cards becomes a card grid; each item's leftover
 * text (the upload time) joins its card's meta line.
 */
function fileLists(root: HTMLElement) {
  for (const list of [...root.querySelectorAll("ul, ol")].reverse()) {
    const items = [...list.children];
    const cards = items.map((item) => {
      const links = item.querySelectorAll("a");
      const card =
        links.length === 1 &&
        links[0].classList.contains("lmsAttachmentCard") &&
        !links[0].classList.contains("lmsImageCard")
          ? links[0]
          : undefined;
      const rest = card
        ? (item.textContent ?? "").replace(card.textContent ?? "", "").trim()
        : "";
      return item.tagName === "LI" && card && rest.length <= 40
        ? { card, rest }
        : undefined;
    });
    if (!items.length || cards.some((card) => !card)) continue;
    const grid = document.createElement("div");
    grid.className = "lmsFileList";
    for (const { card, rest } of cards as {
      card: HTMLAnchorElement;
      rest: string;
    }[]) {
      if (rest) card.querySelector("small")!.textContent += ` · ${rest}`;
      grid.append(card);
    }
    list.replaceWith(grid);
  }
}

export function safeUrl(raw?: string | null): string | undefined {
  try {
    const url = new URL(raw ?? "");
    return url.protocol === "https:" ? url.href : undefined;
  } catch {
    return undefined;
  }
}

export function contentText(html?: string): string {
  if (!html) return "";
  const body = new DOMParser().parseFromString(
    html.slice(0, 100000),
    "text/html",
  ).body;
  body
    .querySelectorAll("script,style,iframe,object,embed,form")
    .forEach((node) => node.remove());
  body.querySelectorAll("br").forEach((node) => node.replaceWith(" "));
  return body.textContent ?? "";
}

export function announcements(html?: string) {
  if (!html) return [];
  const page = new DOMParser().parseFromString(
    html.slice(0, 100000),
    "text/html",
  );
  return [...page.getElementsByTagName("article")].map((article, index) => {
    const titleNode = article.getElementsByTagName("h5")[0];
    const title = titleNode?.textContent?.trim() || t("lms.announcement");
    const time =
      article.getElementsByTagName("time")[0] ||
      (article.getElementsByTagName("p")[0]?.textContent?.match(/^\d{4}年/)
        ? article.getElementsByTagName("p")[0]
        : undefined);
    const date = time?.textContent || "";
    titleNode?.remove();
    time?.remove();
    return {
      id: article.id || String(index),
      title,
      date,
      html: article.innerHTML,
      url: safeUrl(article.getAttribute("data-source-url")),
    };
  });
}

export function formatLmsDate(value: string, locale: string) {
  const parts = value.match(
    /(\d{4})年\s*(\d{1,2})月\s*(\d{1,2})日.*?(\d{1,2}):(\d{2})/,
  );
  if (!parts) return value;
  const [, year, month, day, hour, minute] = parts;
  const date = new Date(
    `${year}-${month.padStart(2, "0")}-${day.padStart(2, "0")}T${hour.padStart(2, "0")}:${minute}:00+09:00`,
  );
  return Number.isNaN(date.getTime())
    ? value
    : formatJst(date, locale, {
        year: "numeric",
        month: "short",
        day: "numeric",
        hour: "2-digit",
        minute: "2-digit",
      });
}
